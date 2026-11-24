//! État du dépôt : `git status --porcelain=v2 -z --branch --untracked-files=all`.

use std::path::PathBuf;

use julie_core::CancelToken;

use crate::error::{Result, VcsError};
use crate::git::{args, Access, Git};
use crate::repo::{Operation, Repository};

/// Où en est `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// Sur une branche. `commit` vaut `None` dans un dépôt sans commit.
    Branch {
        name: String,
        commit: Option<String>,
    },
    /// HEAD détachée sur ce commit.
    Detached { commit: String },
}

impl Head {
    pub fn commit(&self) -> Option<&str> {
        match self {
            Head::Branch { commit, .. } => commit.as_deref(),
            Head::Detached { commit } => Some(commit),
        }
    }

    /// Dépôt sans commit (branche « à naître »).
    pub fn is_unborn(&self) -> bool {
        self.commit().is_none()
    }
}

/// Branche amont de la branche courante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upstream {
    /// `origin/main`.
    pub name: String,
    /// Avance et retard ; `None` si l'amont n'existe plus (« gone »).
    pub ahead_behind: Option<(u32, u32)>,
}

/// Changement d'un côté (index par rapport à HEAD, ou travail par rapport à
/// l'index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Change {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    /// Non fusionné (conflit).
    Unmerged,
}

impl Change {
    fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            b'.' => Change::Unmodified,
            b'M' => Change::Modified,
            b'T' => Change::TypeChanged,
            b'A' => Change::Added,
            b'D' => Change::Deleted,
            b'R' => Change::Renamed,
            b'C' => Change::Copied,
            b'U' => Change::Unmerged,
            other => {
                return Err(VcsError::UnexpectedOutput(format!(
                    "code d'état inconnu {:?}",
                    other as char
                )))
            }
        })
    }
}

/// Nature d'un conflit (lettres `XY` de Git).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConflictKind {
    /// `UU` : modifié des deux côtés.
    BothModified,
    /// `AA` : ajouté des deux côtés (pas de base).
    BothAdded,
    /// `DD` : supprimé des deux côtés.
    BothDeleted,
    /// `AU` : ajouté par nous seulement.
    AddedByUs,
    /// `UA` : ajouté par eux seulement.
    AddedByThem,
    /// `DU` : supprimé par nous, modifié par eux.
    DeletedByUs,
    /// `UD` : modifié par nous, supprimé par eux.
    DeletedByThem,
}

impl ConflictKind {
    fn from_xy(x: u8, y: u8) -> Result<Self> {
        Ok(match (x, y) {
            (b'U', b'U') => ConflictKind::BothModified,
            (b'A', b'A') => ConflictKind::BothAdded,
            (b'D', b'D') => ConflictKind::BothDeleted,
            (b'A', b'U') => ConflictKind::AddedByUs,
            (b'U', b'A') => ConflictKind::AddedByThem,
            (b'D', b'U') => ConflictKind::DeletedByUs,
            (b'U', b'D') => ConflictKind::DeletedByThem,
            _ => {
                return Err(VcsError::UnexpectedOutput(format!(
                    "conflit inconnu {}{}",
                    x as char, y as char
                )))
            }
        })
    }

    /// D'après les étages présents dans l'index (base, nôtre, leur).
    pub fn from_stages(base: bool, ours: bool, theirs: bool) -> Option<Self> {
        Some(match (base, ours, theirs) {
            (true, true, true) => ConflictKind::BothModified,
            (false, true, true) => ConflictKind::BothAdded,
            (true, false, false) => ConflictKind::BothDeleted,
            (false, true, false) => ConflictKind::AddedByUs,
            (false, false, true) => ConflictKind::AddedByThem,
            (true, false, true) => ConflictKind::DeletedByUs,
            (true, true, false) => ConflictKind::DeletedByThem,
            (false, false, false) => return None,
        })
    }

    /// Un côté a supprimé le fichier : pas de fusion à trois volets.
    pub fn involves_deletion(self) -> bool {
        matches!(
            self,
            ConflictKind::BothDeleted | ConflictKind::DeletedByUs | ConflictKind::DeletedByThem
        )
    }
}

/// Résumé d'un fichier pour l'affichage (couleur, groupe).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

/// Groupe de la liste des changements locaux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Conflicted,
    Changed,
    Untracked,
}

/// Un fichier de `git status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// Chemin absolu.
    pub path: PathBuf,
    /// Chemin Git (relatif à la racine du dépôt, séparé par `/`).
    pub repo_path: String,
    /// Ancien chemin d'un renommage ou d'une copie dans l'index.
    pub original_path: Option<PathBuf>,
    pub original_repo_path: Option<String>,
    pub kind: FileKind,
    /// Index par rapport à HEAD (`X`).
    pub staged: Change,
    /// Travail par rapport à l'index (`Y`).
    pub unstaged: Change,
    pub conflict: Option<ConflictKind>,
    /// L'entrée est un sous-module.
    pub submodule: bool,
}

impl FileStatus {
    pub fn group(&self) -> Group {
        match self.kind {
            FileKind::Conflicted => Group::Conflicted,
            FileKind::Untracked => Group::Untracked,
            _ => Group::Changed,
        }
    }
}

/// État complet du dépôt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub head: Head,
    pub upstream: Option<Upstream>,
    pub operation: Option<Operation>,
    /// Dans l'ordre de Git (par chemin).
    pub files: Vec<FileStatus>,
}

impl Status {
    pub fn conflicts(&self) -> impl Iterator<Item = &FileStatus> {
        self.in_group(Group::Conflicted)
    }

    pub fn changed(&self) -> impl Iterator<Item = &FileStatus> {
        self.in_group(Group::Changed)
    }

    pub fn untracked(&self) -> impl Iterator<Item = &FileStatus> {
        self.in_group(Group::Untracked)
    }

    pub fn is_clean(&self) -> bool {
        self.files.is_empty()
    }

    /// Le fichier de ce chemin absolu (nouveau ou ancien chemin d'un renommage).
    pub fn find(&self, path: &std::path::Path) -> Option<&FileStatus> {
        self.files
            .iter()
            .find(|file| file.path == path || file.original_path.as_deref() == Some(path))
    }

    fn in_group(&self, group: Group) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(move |file| file.group() == group)
    }
}

/// Lit l'état du dépôt. **Bloquant.**
pub fn status(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Status> {
    let output = git.run_ok(
        repo.work_tree(),
        &args([
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
        ]),
        Access::Read,
        None,
        cancel,
    )?;
    let mut status = parse_status(&output.stdout, repo)?;
    status.operation = repo.operation();
    Ok(status)
}

fn text(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes)
        .map_err(|_| VcsError::UnexpectedOutput("chemin qui n'est pas de l'UTF-8".into()))
}

fn bad(record: &str) -> VcsError {
    VcsError::UnexpectedOutput(format!("ligne d'état illisible : {record:?}"))
}

/// Analyse la sortie de `git status --porcelain=v2 -z --branch` (sans
/// l'opération en cours, lue à part). Fonction pure.
pub fn parse_status(output: &[u8], repo: &Repository) -> Result<Status> {
    let mut head_name: Option<String> = None;
    let mut head_commit: Option<String> = None;
    let mut upstream: Option<Upstream> = None;
    let mut files = Vec::new();

    let mut records = output.split(|&byte| byte == 0);
    while let Some(raw) = records.next() {
        if raw.is_empty() {
            continue;
        }
        let record = text(raw)?;
        let tag = record.as_bytes()[0];
        match tag {
            b'#' => {
                let mut parts = record.splitn(3, ' ');
                let _ = parts.next();
                let key = parts.next().unwrap_or_default();
                let value = parts.next().unwrap_or_default();
                match key {
                    "branch.oid" => {
                        head_commit = (value != "(initial)").then(|| value.to_owned());
                    }
                    "branch.head" => head_name = Some(value.to_owned()),
                    "branch.upstream" => {
                        upstream = Some(Upstream {
                            name: value.to_owned(),
                            ahead_behind: None,
                        })
                    }
                    "branch.ab" => {
                        let mut numbers = value
                            .split(' ')
                            .map(|part| part.trim_start_matches(['+', '-']).parse::<u32>());
                        if let (Some(Ok(ahead)), Some(Ok(behind)), Some(upstream)) =
                            (numbers.next(), numbers.next(), upstream.as_mut())
                        {
                            upstream.ahead_behind = Some((ahead, behind));
                        }
                    }
                    _ => {}
                }
            }
            b'1' | b'2' => {
                let fields = if tag == b'1' { 9 } else { 10 };
                let parts: Vec<&str> = record.splitn(fields, ' ').collect();
                if parts.len() != fields || parts[1].len() != 2 {
                    return Err(bad(record));
                }
                let xy = parts[1].as_bytes();
                let staged = Change::from_code(xy[0])?;
                let unstaged = Change::from_code(xy[1])?;
                let submodule = parts[2].starts_with('S');
                let repo_path = parts[fields - 1].to_owned();
                let original_repo_path = if tag == b'2' {
                    let original = records.next().ok_or_else(|| bad(record))?;
                    Some(text(original)?.to_owned())
                } else {
                    None
                };
                let kind = summarize(staged, unstaged);
                files.push(FileStatus {
                    path: repo.to_absolute(&repo_path),
                    original_path: original_repo_path.as_deref().map(|p| repo.to_absolute(p)),
                    repo_path,
                    original_repo_path,
                    kind,
                    staged,
                    unstaged,
                    conflict: None,
                    submodule,
                });
            }
            b'u' => {
                let parts: Vec<&str> = record.splitn(11, ' ').collect();
                if parts.len() != 11 || parts[1].len() != 2 {
                    return Err(bad(record));
                }
                let xy = parts[1].as_bytes();
                let repo_path = parts[10].to_owned();
                files.push(FileStatus {
                    path: repo.to_absolute(&repo_path),
                    repo_path,
                    original_path: None,
                    original_repo_path: None,
                    kind: FileKind::Conflicted,
                    staged: Change::Unmerged,
                    unstaged: Change::Unmerged,
                    conflict: Some(ConflictKind::from_xy(xy[0], xy[1])?),
                    submodule: parts[2].starts_with('S'),
                });
            }
            b'?' => {
                let repo_path = record.get(2..).ok_or_else(|| bad(record))?.to_owned();
                files.push(FileStatus {
                    path: repo.to_absolute(&repo_path),
                    repo_path,
                    original_path: None,
                    original_repo_path: None,
                    kind: FileKind::Untracked,
                    staged: Change::Unmodified,
                    unstaged: Change::Unmodified,
                    conflict: None,
                    submodule: false,
                });
            }
            // Ignorés : pas montrés.
            b'!' => {}
            _ => return Err(bad(record)),
        }
    }

    let head = match head_name.as_deref() {
        Some("(detached)") => Head::Detached {
            commit: head_commit.ok_or_else(|| bad("HEAD détachée sans commit"))?,
        },
        Some(name) => Head::Branch {
            name: name.to_owned(),
            commit: head_commit,
        },
        None => return Err(bad("pas de ligne branch.head")),
    };
    Ok(Status {
        head,
        upstream,
        operation: None,
        files,
    })
}

fn summarize(staged: Change, unstaged: Change) -> FileKind {
    use Change::*;
    if staged == Renamed {
        return FileKind::Renamed;
    }
    if staged == Copied {
        return FileKind::Copied;
    }
    if staged == Deleted || unstaged == Deleted {
        // « AD » : ajouté puis supprimé du disque ; il n'existe nulle part
        // ailleurs que dans l'index, on le montre comme ajouté.
        return if staged == Added {
            FileKind::Added
        } else {
            FileKind::Deleted
        };
    }
    if staged == Added {
        return FileKind::Added;
    }
    if staged == TypeChanged || unstaged == TypeChanged {
        return FileKind::TypeChanged;
    }
    FileKind::Modified
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repository {
        // Passe par l'API publique de conversion seulement.
        crate::repo::tests_support::repository(r"C:\depot")
    }

    #[test]
    fn parses_every_kind_of_record() {
        let mut out = Vec::new();
        for record in [
            "# branch.oid 1111111111111111111111111111111111111111",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 .M N... 100644 100644 100644 aaaa aaaa src/a b.php",
            "1 A. N... 000000 100644 100644 0000 bbbb nouveau é.txt",
            "1 .D N... 100644 100644 000000 cccc cccc supprimé.txt",
            "2 R. N... 100644 100644 100644 dddd dddd R100 nouveau nom.txt",
            "ancien nom.txt",
            "u UU N... 100644 100644 100644 100644 e1 e2 e3 conflit.txt",
            "u UD N... 100644 100644 000000 100644 e1 e2 0000 supprimé par eux.txt",
            "1 .M SC.. 160000 160000 160000 ffff ffff module",
            "? non suivi/x.txt",
            "! ignoré.log",
        ] {
            out.extend_from_slice(record.as_bytes());
            out.push(0);
        }
        let status = parse_status(&out, &repo()).unwrap();
        assert_eq!(
            status.head,
            Head::Branch {
                name: "main".into(),
                commit: Some("1".repeat(40))
            }
        );
        assert_eq!(
            status.upstream,
            Some(Upstream {
                name: "origin/main".into(),
                ahead_behind: Some((2, 1))
            })
        );
        let kinds: Vec<_> = status
            .files
            .iter()
            .map(|f| (f.repo_path.as_str(), f.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("src/a b.php", FileKind::Modified),
                ("nouveau é.txt", FileKind::Added),
                ("supprimé.txt", FileKind::Deleted),
                ("nouveau nom.txt", FileKind::Renamed),
                ("conflit.txt", FileKind::Conflicted),
                ("supprimé par eux.txt", FileKind::Conflicted),
                ("module", FileKind::Modified),
                ("non suivi/x.txt", FileKind::Untracked),
            ]
        );
        let renamed = &status.files[3];
        assert_eq!(
            renamed.original_repo_path.as_deref(),
            Some("ancien nom.txt")
        );
        assert_eq!(
            renamed.original_path,
            Some(PathBuf::from(r"C:\depot\ancien nom.txt"))
        );
        assert_eq!(status.files[4].conflict, Some(ConflictKind::BothModified));
        assert_eq!(status.files[5].conflict, Some(ConflictKind::DeletedByThem));
        assert!(status.files[6].submodule);
        assert_eq!(status.files[0].path, PathBuf::from(r"C:\depot\src\a b.php"));
        assert_eq!(status.conflicts().count(), 2);
        assert_eq!(status.changed().count(), 5);
        assert_eq!(status.untracked().count(), 1);
    }

    #[test]
    fn unborn_detached_and_gone_upstream() {
        let unborn =
            parse_status(b"# branch.oid (initial)\0# branch.head main\0", &repo()).unwrap();
        assert!(unborn.head.is_unborn());
        let detached =
            parse_status(b"# branch.oid abc\0# branch.head (detached)\0", &repo()).unwrap();
        assert_eq!(
            detached.head,
            Head::Detached {
                commit: "abc".into()
            }
        );
        let gone = parse_status(
            b"# branch.oid abc\0# branch.head main\0# branch.upstream origin/x\0",
            &repo(),
        )
        .unwrap();
        assert_eq!(gone.upstream.unwrap().ahead_behind, None);
    }

    #[test]
    fn garbage_is_refused() {
        assert!(parse_status(b"# branch.head main\0Z what\0", &repo()).is_err());
        assert!(parse_status(b"# branch.head main\x001 .M\0", &repo()).is_err());
        assert!(parse_status(b"", &repo()).is_err());
    }

    #[test]
    fn conflict_kind_from_stages_matches_git_letters() {
        assert_eq!(
            ConflictKind::from_stages(true, true, false),
            Some(ConflictKind::DeletedByThem)
        );
        assert_eq!(
            ConflictKind::from_stages(false, true, true),
            Some(ConflictKind::BothAdded)
        );
        assert_eq!(ConflictKind::from_stages(false, false, false), None);
        assert!(ConflictKind::DeletedByUs.involves_deletion());
        assert!(!ConflictKind::BothAdded.involves_deletion());
    }
}
