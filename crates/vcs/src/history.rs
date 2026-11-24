//! Journal : liste paginée des commits, détail d'un commit et de ses fichiers.

use std::path::PathBuf;

use julie_core::CancelToken;

use crate::content::{check_revision, has_head};
use crate::error::{Result, VcsError};
use crate::git::{args, Access, Git};
use crate::repo::Repository;
use crate::status::Change;

/// Taille de page conseillée (`git.md` : lots de 500).
pub const DEFAULT_PAGE_SIZE: usize = 500;

/// Champs lus pour chaque commit, séparés par NUL (sortie `-z`).
const SUMMARY_FORMAT: &str = "%H%x00%h%x00%P%x00%an%x00%ae%x00%ad%x00%cn%x00%ce%x00%cd%x00%D%x00%s";
const SUMMARY_FIELDS: usize = 11;

/// Une page du journal à charger.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogQuery {
    /// Révision de départ ; `None` : `HEAD`.
    pub revision: Option<String>,
    /// Commits à sauter (pagination).
    pub skip: usize,
    /// Commits au plus dans la page.
    pub limit: usize,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            revision: None,
            skip: 0,
            limit: DEFAULT_PAGE_SIZE,
        }
    }
}

impl LogQuery {
    /// La page suivante de celle-ci.
    pub fn next_page(&self) -> Self {
        Self {
            skip: self.skip + self.limit,
            ..self.clone()
        }
    }
}

/// Auteur ou validateur, avec sa date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    /// Secondes depuis l'époque Unix.
    pub time: i64,
    /// Décalage horaire de la personne, en minutes (`+0100` → 60).
    pub offset_minutes: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefKind {
    /// `HEAD` (détachée, ou pointant sur la branche qui suit dans la liste).
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Other,
}

/// Une référence affichée à côté d'un commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    pub kind: RefKind,
    /// Nom court : `main`, `origin/main`, `v1.0`, `HEAD`.
    pub name: String,
    /// Branche courante (`HEAD -> main`).
    pub current: bool,
}

/// Une ligne du journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    pub id: String,
    pub short_id: String,
    pub parents: Vec<String>,
    pub author: Person,
    pub committer: Person,
    pub subject: String,
    pub refs: Vec<RefLabel>,
}

impl CommitSummary {
    /// Commit de fusion (plusieurs parents) : affiché grisé.
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// Une page du journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPage {
    pub query: LogQuery,
    pub commits: Vec<CommitSummary>,
    /// D'autres commits suivent cette page.
    pub has_more: bool,
}

/// Un fichier changé par un commit (par rapport à son premier parent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    pub path: PathBuf,
    pub repo_path: String,
    pub original_path: Option<PathBuf>,
    pub original_repo_path: Option<String>,
    pub change: Change,
    /// Le premier parent (volet gauche du diff) ; `None` pour un commit racine.
    pub parent: Option<String>,
}

/// Détail d'un commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetails {
    pub summary: CommitSummary,
    /// Message complet, sans le saut de ligne final.
    pub message: String,
    pub files: Vec<CommitFile>,
}

/// Charge une page du journal. Dépôt sans commit : page vide. **Bloquant.**
pub fn log(
    git: &Git,
    repo: &Repository,
    query: &LogQuery,
    cancel: &CancelToken,
) -> Result<LogPage> {
    let revision = query.revision.clone().unwrap_or_else(|| "HEAD".to_owned());
    check_revision(&revision)?;
    if query.revision.is_none() && !has_head(git, repo, cancel)? {
        return Ok(LogPage {
            query: query.clone(),
            commits: Vec::new(),
            has_more: false,
        });
    }
    let output = git.run_ok(
        repo.work_tree(),
        &args([
            "log".to_owned(),
            "-z".to_owned(),
            "--date=raw".to_owned(),
            "--decorate=full".to_owned(),
            format!("--format={SUMMARY_FORMAT}"),
            format!("--skip={}", query.skip),
            format!("--max-count={}", query.limit.saturating_add(1)),
            "--end-of-options".to_owned(),
            revision,
            "--".to_owned(),
        ]),
        Access::Read,
        None,
        cancel,
    )?;
    let mut commits = parse_summaries(&output.stdout)?;
    let has_more = commits.len() > query.limit;
    commits.truncate(query.limit);
    Ok(LogPage {
        query: query.clone(),
        commits,
        has_more,
    })
}

/// Détail d'un commit et de ses fichiers. **Bloquant.**
pub fn commit_details(
    git: &Git,
    repo: &Repository,
    commit: &str,
    cancel: &CancelToken,
) -> Result<CommitDetails> {
    check_revision(commit)?;
    let output = git.run_ok(
        repo.work_tree(),
        &args([
            "log".to_owned(),
            "-1".to_owned(),
            "-z".to_owned(),
            "--date=raw".to_owned(),
            "--decorate=full".to_owned(),
            format!("--format={SUMMARY_FORMAT}%x00%B"),
            "--end-of-options".to_owned(),
            commit.to_owned(),
            "--".to_owned(),
        ]),
        Access::Read,
        None,
        cancel,
    )?;
    let text = utf8(&output.stdout)?;
    let fields: Vec<&str> = text.split('\0').collect();
    if fields.len() < SUMMARY_FIELDS + 1 {
        return Err(VcsError::UnexpectedOutput(format!("log -1 : {text:?}")));
    }
    let summary = summary_from(&fields[..SUMMARY_FIELDS])?;
    let message = fields[SUMMARY_FIELDS].trim_end_matches('\n').to_owned();

    let parent = summary.parents.first().cloned();
    let mut command = vec![
        "diff-tree",
        "-r",
        "-z",
        "-M",
        "--name-status",
        "--no-commit-id",
    ];
    if parent.is_none() {
        command.push("--root");
    }
    let mut command = args(command);
    if let Some(parent) = &parent {
        command.push(parent.into());
    }
    command.push(summary.id.as_str().into());
    let output = git.run_ok(repo.work_tree(), &command, Access::Read, None, cancel)?;
    let files = parse_name_status(&output.stdout, repo, parent.as_deref())?;
    Ok(CommitDetails {
        summary,
        message,
        files,
    })
}

fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| VcsError::UnexpectedOutput("journal : pas de l'UTF-8".into()))
}

/// Analyse la sortie de `git log -z --format=SUMMARY_FORMAT`. Fonction pure.
pub(crate) fn parse_summaries(output: &[u8]) -> Result<Vec<CommitSummary>> {
    let text = utf8(output)?;
    let mut fields: Vec<&str> = text.split('\0').collect();
    // Chaque commit se termine par NUL : le dernier morceau est vide.
    if fields.last() == Some(&"") {
        fields.pop();
    }
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    if !fields.len().is_multiple_of(SUMMARY_FIELDS) {
        return Err(VcsError::UnexpectedOutput(format!(
            "journal : {} champs",
            fields.len()
        )));
    }
    fields.chunks(SUMMARY_FIELDS).map(summary_from).collect()
}

fn summary_from(fields: &[&str]) -> Result<CommitSummary> {
    // Entre deux commits, `-z` met un NUL ; git peut aussi y laisser un `\n`.
    let id = fields[0].trim_start_matches('\n').to_owned();
    if id.is_empty() {
        return Err(VcsError::UnexpectedOutput(
            "journal : commit sans hash".into(),
        ));
    }
    Ok(CommitSummary {
        id,
        short_id: fields[1].to_owned(),
        parents: fields[2].split_whitespace().map(str::to_owned).collect(),
        author: person(fields[3], fields[4], fields[5])?,
        committer: person(fields[6], fields[7], fields[8])?,
        refs: parse_refs(fields[9]),
        subject: fields[10].to_owned(),
    })
}

fn person(name: &str, email: &str, raw_date: &str) -> Result<Person> {
    let bad = || VcsError::UnexpectedOutput(format!("date {raw_date:?}"));
    let mut parts = raw_date.split(' ');
    let time: i64 = parts.next().and_then(|t| t.parse().ok()).ok_or_else(bad)?;
    let zone = parts.next().unwrap_or("+0000");
    let (sign, digits) = zone.split_at(1.min(zone.len()));
    let sign = if sign == "-" { -1 } else { 1 };
    let hours: i32 = digits
        .get(..2)
        .and_then(|h| h.parse().ok())
        .ok_or_else(bad)?;
    let minutes: i32 = digits
        .get(2..4)
        .and_then(|m| m.parse().ok())
        .ok_or_else(bad)?;
    Ok(Person {
        name: name.to_owned(),
        email: email.to_owned(),
        time,
        offset_minutes: sign * (hours * 60 + minutes),
    })
}

/// `%D` avec `--decorate=full` : `HEAD -> refs/heads/main, tag: refs/tags/v1, refs/remotes/origin/main`.
pub(crate) fn parse_refs(decorations: &str) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    for item in decorations.split(", ").filter(|item| !item.is_empty()) {
        if let Some(target) = item.strip_prefix("HEAD -> ") {
            labels.push(RefLabel {
                kind: RefKind::Head,
                name: "HEAD".into(),
                current: false,
            });
            let mut label = ref_label(target);
            label.current = true;
            labels.push(label);
        } else if let Some(tag) = item.strip_prefix("tag: ") {
            let mut label = ref_label(tag);
            label.kind = RefKind::Tag;
            labels.push(label);
        } else if item == "HEAD" {
            labels.push(RefLabel {
                kind: RefKind::Head,
                name: "HEAD".into(),
                current: false,
            });
        } else {
            labels.push(ref_label(item));
        }
    }
    labels
}

fn ref_label(full: &str) -> RefLabel {
    let (kind, name) = if let Some(name) = full.strip_prefix("refs/heads/") {
        (RefKind::LocalBranch, name)
    } else if let Some(name) = full.strip_prefix("refs/remotes/") {
        (RefKind::RemoteBranch, name)
    } else if let Some(name) = full.strip_prefix("refs/tags/") {
        (RefKind::Tag, name)
    } else {
        (RefKind::Other, full)
    };
    RefLabel {
        kind,
        name: name.to_owned(),
        current: false,
    }
}

/// Sortie de `git diff-tree -r -z --name-status`. Fonction pure.
pub(crate) fn parse_name_status(
    output: &[u8],
    repo: &Repository,
    parent: Option<&str>,
) -> Result<Vec<CommitFile>> {
    let text = utf8(output)?;
    let mut tokens = text.split('\0').filter(|token| !token.is_empty());
    let mut files = Vec::new();
    while let Some(code) = tokens.next() {
        let letter = code.as_bytes()[0];
        let change = match letter {
            b'M' => Change::Modified,
            b'A' => Change::Added,
            b'D' => Change::Deleted,
            b'T' => Change::TypeChanged,
            b'R' => Change::Renamed,
            b'C' => Change::Copied,
            b'U' => Change::Unmerged,
            _ => {
                return Err(VcsError::UnexpectedOutput(format!(
                    "diff-tree : code {code:?}"
                )))
            }
        };
        let missing = || VcsError::UnexpectedOutput("diff-tree : chemin manquant".into());
        let (original, repo_path) = if matches!(change, Change::Renamed | Change::Copied) {
            let original = tokens.next().ok_or_else(missing)?;
            (
                Some(original.to_owned()),
                tokens.next().ok_or_else(missing)?,
            )
        } else {
            (None, tokens.next().ok_or_else(missing)?)
        };
        files.push(CommitFile {
            path: repo.to_absolute(repo_path),
            repo_path: repo_path.to_owned(),
            original_path: original.as_deref().map(|p| repo.to_absolute(p)),
            original_repo_path: original,
            change,
            parent: parent.map(str::to_owned),
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorations_are_split_by_kind() {
        let labels = parse_refs(
            "HEAD -> refs/heads/main, tag: refs/tags/v1.0, refs/remotes/origin/main, refs/stash",
        );
        assert_eq!(
            labels,
            vec![
                RefLabel {
                    kind: RefKind::Head,
                    name: "HEAD".into(),
                    current: false
                },
                RefLabel {
                    kind: RefKind::LocalBranch,
                    name: "main".into(),
                    current: true
                },
                RefLabel {
                    kind: RefKind::Tag,
                    name: "v1.0".into(),
                    current: false
                },
                RefLabel {
                    kind: RefKind::RemoteBranch,
                    name: "origin/main".into(),
                    current: false
                },
                RefLabel {
                    kind: RefKind::Other,
                    name: "refs/stash".into(),
                    current: false
                },
            ]
        );
        assert!(parse_refs("").is_empty());
    }

    #[test]
    fn summaries_are_parsed_by_groups_of_fields() {
        let one = "aaaa\0aa\0p1 p2\0Ana\0ana@x\x001700000000 +0130\0Bob\0bob@x\x001700000001 -0500\0\0Fusion\0";
        let commits = parse_summaries(format!("{one}\n{one}").as_bytes()).unwrap();
        assert_eq!(commits.len(), 2);
        let first = &commits[0];
        assert!(first.is_merge());
        assert_eq!(first.author.offset_minutes, 90);
        assert_eq!(first.committer.offset_minutes, -300);
        assert_eq!(first.subject, "Fusion");
        assert_eq!(commits[1].id, "aaaa");
        assert!(parse_summaries(b"").unwrap().is_empty());
        assert!(parse_summaries(b"a\0b\0").is_err());
    }

    #[test]
    fn name_status_reads_renames() {
        let repo = crate::repo::tests_support::repository(r"C:\d");
        let files = parse_name_status(
            "M\0a.txt\0R087\0vieux nom.txt\0nouveau nom.txt\0A\0dossier/é.txt\0".as_bytes(),
            &repo,
            Some("p"),
        )
        .unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[1].change, Change::Renamed);
        assert_eq!(
            files[1].original_repo_path.as_deref(),
            Some("vieux nom.txt")
        );
        assert_eq!(files[1].repo_path, "nouveau nom.txt");
        assert_eq!(files[2].path, PathBuf::from(r"C:\d\dossier\é.txt"));
        assert_eq!(files[0].parent.as_deref(), Some("p"));
    }
}
