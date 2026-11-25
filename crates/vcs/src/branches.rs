//! Branches : liste (`for-each-ref`), récentes (reflog), nom valable, créer,
//! renommer, supprimer, basculer (avec mise de côté ou abandon nommé des
//! changements locaux).

use std::ffi::OsString;
use std::path::PathBuf;

use julie_core::CancelToken;

use crate::content::{check_revision, has_head};
use crate::error::{BranchNameProblem, OverwriteConflict, Result, VcsError};
use crate::git::{args, failure, Access, Git};
use crate::repo::{current_branch_name, Repository};
use crate::stash::{parse_overwrite, with_local_changes, AutoStash, LocalChanges, StashReason};

/// Nombre de branches récentes rendues.
pub const RECENT_BRANCHES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchKind {
    Local,
    Remote,
}

/// Amont d'une branche locale et position par rapport à lui (calculée par
/// `for-each-ref`, sans réseau : d'après le dernier fetch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracking {
    /// `origin/feature/login`.
    pub upstream: String,
    /// Commits locaux absents de l'amont (« ↑ »).
    pub ahead: u32,
    /// Commits de l'amont absents localement (« ↓ »).
    pub behind: u32,
    /// L'amont n'existe plus sur le distant (`[gone]`, « amont supprimé »).
    pub gone: bool,
}

/// Dernier commit d'une branche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCommit {
    pub id: String,
    pub subject: String,
    pub author: String,
    /// Date du commit (secondes Unix).
    pub time: i64,
}

/// Une branche locale ou distante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// Nom affiché : `feature/login`, ou `origin/feature/login` pour une
    /// distante.
    pub name: String,
    /// Référence complète : `refs/heads/feature/login`.
    pub full_ref: String,
    pub kind: BranchKind,
    /// Distante : le nom du distant (`origin`).
    pub remote: Option<String>,
    /// Nom sans le distant (`feature/login`) ; égal à `name` pour une locale.
    pub short_name: String,
    /// Branche courante.
    pub current: bool,
    pub tracking: Option<Tracking>,
    pub last_commit: BranchCommit,
}

/// Toutes les branches du dépôt, pour la fenêtre surgissante et l'arbre du
/// journal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchList {
    /// Branche courante, née ou à naître (dépôt sans commit) ; `None` si HEAD
    /// est détachée.
    pub current: Option<String>,
    /// HEAD détachée : son commit.
    pub detached_at: Option<String>,
    /// Dépôt sans commit.
    pub unborn: bool,
    /// Locales, par nom.
    pub local: Vec<Branch>,
    /// Distantes, par nom ; `origin/HEAD` n'y est pas.
    pub remote: Vec<Branch>,
    /// Les distants configurés (`git remote`).
    pub remotes: Vec<String>,
    /// Les dernières branches locales où l'on a basculé (reflog de HEAD, la
    /// courante exclue), de la plus récente à la plus ancienne ; 5 au plus.
    pub recent: Vec<String>,
}

impl BranchList {
    pub fn find_local(&self, name: &str) -> Option<&Branch> {
        self.local.iter().find(|branch| branch.name == name)
    }

    pub fn find_remote(&self, name: &str) -> Option<&Branch> {
        self.remote.iter().find(|branch| branch.name == name)
    }

    /// La branche courante (absente si à naître ou détachée).
    pub fn current_branch(&self) -> Option<&Branch> {
        self.local.iter().find(|branch| branch.current)
    }
}

const BRANCH_FORMAT: &str = "%(refname)%00%(objectname)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(HEAD)%00%(symref)%00%(committerdate:unix)%00%(authorname)%00%(contents:subject)";
const BRANCH_FIELDS: usize = 9;

/// Liste les branches, les distants et les branches récentes. **Bloquant.**
/// Quatre commandes de lecture, aucune réseau.
pub fn list_branches(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<BranchList> {
    let remotes = list_remotes(git, repo, cancel)?;
    let text = git.read(
        repo.work_tree(),
        &args([
            "for-each-ref".to_owned(),
            format!("--format={BRANCH_FORMAT}"),
            "refs/heads".to_owned(),
            "refs/remotes".to_owned(),
        ]),
        cancel,
    )?;
    let mut list = parse_branches(&text, &remotes)?;
    list.remotes = remotes;
    list.current = current_branch_name(repo.git_dir());
    list.unborn = !has_head(git, repo, cancel)?;
    if list.current.is_none() {
        list.detached_at = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    }
    if !list.unborn {
        list.recent = recent_branches(git, repo, &list, cancel)?;
    }
    Ok(list)
}

/// Les distants configurés. **Bloquant.**
pub fn list_remotes(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Vec<String>> {
    let text = git.read(repo.work_tree(), &args(["remote"]), cancel)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Analyse la sortie de `for-each-ref --format=BRANCH_FORMAT`. Fonction pure.
pub(crate) fn parse_branches(text: &str, remotes: &[String]) -> Result<BranchList> {
    let mut list = BranchList::default();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let fields: Vec<&str> = line.splitn(BRANCH_FIELDS, '\0').collect();
        if fields.len() != BRANCH_FIELDS {
            return Err(VcsError::UnexpectedOutput(format!(
                "for-each-ref : {line:?}"
            )));
        }
        // `origin/HEAD` et autres références symboliques : pas des branches.
        if !fields[5].is_empty() {
            continue;
        }
        let full_ref = fields[0].to_owned();
        let last_commit = BranchCommit {
            id: fields[1].to_owned(),
            subject: fields[8].to_owned(),
            author: fields[7].to_owned(),
            time: fields[6].trim().parse().unwrap_or_default(),
        };
        if let Some(name) = full_ref.strip_prefix("refs/heads/") {
            let tracking = (!fields[2].is_empty()).then(|| parse_track(fields[2], fields[3]));
            list.local.push(Branch {
                name: name.to_owned(),
                short_name: name.to_owned(),
                full_ref: full_ref.clone(),
                kind: BranchKind::Local,
                remote: None,
                current: fields[4] == "*",
                tracking,
                last_commit,
            });
        } else if let Some(name) = full_ref.strip_prefix("refs/remotes/") {
            let (remote, short) = split_remote(name, remotes);
            list.remote.push(Branch {
                name: name.to_owned(),
                short_name: short.to_owned(),
                full_ref: full_ref.clone(),
                kind: BranchKind::Remote,
                remote: Some(remote.to_owned()),
                current: false,
                tracking: None,
                last_commit,
            });
        }
    }
    Ok(list)
}

/// `origin/feature/x` → (`origin`, `feature/x`), en prenant le plus long nom de
/// distant qui convient (un distant peut contenir `/`).
pub(crate) fn split_remote<'a>(name: &'a str, remotes: &[String]) -> (&'a str, &'a str) {
    let best = remotes
        .iter()
        .filter(|remote| {
            name.len() > remote.len()
                && name.starts_with(remote.as_str())
                && name.as_bytes()[remote.len()] == b'/'
        })
        .max_by_key(|remote| remote.len());
    match best {
        Some(remote) => (&name[..remote.len()], &name[remote.len() + 1..]),
        None => name.split_once('/').unwrap_or((name, "")),
    }
}

/// `ahead 1, behind 2`, `ahead 3`, `behind 1`, `gone`, ou vide (à jour).
fn parse_track(upstream: &str, track: &str) -> Tracking {
    let mut tracking = Tracking {
        upstream: upstream.to_owned(),
        ahead: 0,
        behind: 0,
        gone: false,
    };
    for part in track.split(',').map(str::trim) {
        if part == "gone" {
            tracking.gone = true;
        } else if let Some(n) = part.strip_prefix("ahead ") {
            tracking.ahead = n.trim().parse().unwrap_or_default();
        } else if let Some(n) = part.strip_prefix("behind ") {
            tracking.behind = n.trim().parse().unwrap_or_default();
        }
    }
    tracking
}

/// Les branches où l'on a basculé, d'après le reflog de HEAD (`checkout:
/// moving from A to B`) : bascules de l'IDE comme de la ligne de commande.
fn recent_branches(
    git: &Git,
    repo: &Repository,
    list: &BranchList,
    cancel: &CancelToken,
) -> Result<Vec<String>> {
    let output = git.run(
        repo.work_tree(),
        &args([
            "log",
            "-g",
            "-z",
            "--format=%gs",
            "--grep-reflog=checkout: moving from ",
            "--max-count=500",
            "HEAD",
            "--",
        ]),
        Access::Read,
        None,
        cancel,
    )?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(parse_recent(
        &output.stdout_text(),
        list.current.as_deref(),
        |name| list.find_local(name).is_some(),
    ))
}

/// Fonction pure : les cibles des bascules, sans doublon, la courante et les
/// branches disparues exclues.
pub(crate) fn parse_recent(
    reflog: &str,
    current: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut recent: Vec<String> = Vec::new();
    for entry in reflog.split('\0').map(|e| e.trim_matches('\n')) {
        let Some(rest) = entry.strip_prefix("checkout: moving from ") else {
            continue;
        };
        let Some((_, target)) = rest.rsplit_once(" to ") else {
            continue;
        };
        let target = target.trim();
        if Some(target) == current || !exists(target) || recent.iter().any(|r| r == target) {
            continue;
        }
        recent.push(target.to_owned());
        if recent.len() == RECENT_BRANCHES {
            break;
        }
    }
    recent
}

/// Un nœud de l'arbre des branches (dossiers par préfixe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchNode {
    /// Dossier `feature` (au moins deux branches dedans).
    Folder {
        /// Nom du dossier (`feature`).
        name: String,
        /// Chemin complet du dossier (`team/feature`).
        path: String,
        children: Vec<BranchNode>,
    },
    /// Une branche. `label` : ce qui s'affiche à ce niveau (un dossier à un
    /// seul élément est replié : `bugfix/crash` en une ligne).
    Branch { label: String, index: usize },
}

/// Range des noms de branches (sans le distant) en dossiers par préfixe, dans
/// l'ordre donné. `index` renvoie à la position dans `names`. Fonction pure.
pub fn branch_tree<S: AsRef<str>>(names: &[S]) -> Vec<BranchNode> {
    let items: Vec<(usize, &str)> = names
        .iter()
        .enumerate()
        .map(|(index, name)| (index, name.as_ref()))
        .collect();
    tree_level(&items, "")
}

fn tree_level(items: &[(usize, &str)], prefix: &str) -> Vec<BranchNode> {
    let mut groups: Vec<(&str, Vec<(usize, &str)>)> = Vec::new();
    let mut nodes_order: Vec<Result<usize, usize>> = Vec::new(); // Ok(groupe) / Err(branche)
    let mut singles: Vec<(usize, &str)> = Vec::new();
    for &(index, name) in items {
        match name.split_once('/') {
            Some((head, rest)) => match groups.iter().position(|(h, _)| *h == head) {
                Some(position) => groups[position].1.push((index, rest)),
                None => {
                    nodes_order.push(Ok(groups.len()));
                    groups.push((head, vec![(index, rest)]));
                }
            },
            None => {
                nodes_order.push(Err(singles.len()));
                singles.push((index, name));
            }
        }
    }
    nodes_order
        .into_iter()
        .map(|slot| match slot {
            Err(position) => {
                let (index, name) = singles[position];
                BranchNode::Branch {
                    label: name.to_owned(),
                    index,
                }
            }
            Ok(position) => {
                let (head, children) = &groups[position];
                if children.len() == 1 {
                    let (index, rest) = children[0];
                    BranchNode::Branch {
                        label: format!("{head}/{rest}"),
                        index,
                    }
                } else {
                    let path = if prefix.is_empty() {
                        (*head).to_owned()
                    } else {
                        format!("{prefix}/{head}")
                    };
                    BranchNode::Folder {
                        name: (*head).to_owned(),
                        children: tree_level(children, &path),
                        path,
                    }
                }
            }
        })
        .collect()
}

// ------------------------------------------------------------------ noms

/// Le nom avec ses espaces remplacés par des tirets (« ma branche » →
/// « ma-branche »), s'il diffère. Fonction pure.
pub fn suggest_branch_name(name: &str) -> Option<String> {
    let suggestion = name.split_whitespace().collect::<Vec<_>>().join("-");
    (!suggestion.is_empty() && suggestion != name).then_some(suggestion)
}

/// Collision avec les branches locales existantes, casse comprise (Windows ne
/// distingue pas `Feature/x` de `feature/x`). Fonction pure.
pub fn branch_name_collision(name: &str, existing: &[String]) -> Option<BranchNameProblem> {
    let lower = name.to_lowercase();
    for other in existing {
        if other == name {
            return Some(BranchNameProblem::AlreadyExists(other.clone()));
        }
        let other_lower = other.to_lowercase();
        if other_lower == lower {
            return Some(BranchNameProblem::SameNameDifferentCase(other.clone()));
        }
        if other_lower.starts_with(&format!("{lower}/")) {
            return Some(BranchNameProblem::FolderOfExisting(other.clone()));
        }
        if lower.starts_with(&format!("{other_lower}/")) {
            return Some(BranchNameProblem::InsideExisting(other.clone()));
        }
        // Un dossier commun écrit avec une autre casse.
        let mine: Vec<&str> = name.split('/').collect();
        let theirs: Vec<&str> = other.split('/').collect();
        for (a, b) in mine
            .iter()
            .zip(&theirs)
            .take(mine.len().min(theirs.len()) - 1)
        {
            if a == b {
                continue;
            }
            if a.to_lowercase() == b.to_lowercase() {
                return Some(BranchNameProblem::FolderCaseMismatch(other.clone()));
            }
            break;
        }
    }
    None
}

/// Vérifie un nom de branche à créer : syntaxe (`git check-ref-format
/// --branch`) et collisions avec les branches locales. `None` : valable.
/// `ignore` : une branche à ne pas compter (celle qu'on renomme). **Bloquant**
/// (deux commandes de lecture) ; à relancer à la frappe dans une tâche.
pub fn check_branch_name(
    git: &Git,
    repo: &Repository,
    name: &str,
    ignore: Option<&str>,
    cancel: &CancelToken,
) -> Result<Option<BranchNameProblem>> {
    if name.trim().is_empty() {
        return Ok(Some(BranchNameProblem::Empty));
    }
    let invalid = || BranchNameProblem::Invalid {
        suggestion: suggest_branch_name(name).filter(|s| syntax_ok_pure(s)),
    };
    if !syntax_ok_pure(name) {
        return Ok(Some(invalid()));
    }
    let output = git.run(
        repo.work_tree(),
        &args(["check-ref-format", "--branch", name]),
        Access::Read,
        None,
        cancel,
    )?;
    if !output.status.success() || output.stdout_text().trim() != name {
        return Ok(Some(invalid()));
    }
    let existing: Vec<String> = local_branch_names(git, repo, cancel)?
        .into_iter()
        .filter(|other| Some(other.as_str()) != ignore)
        .collect();
    Ok(branch_name_collision(name, &existing))
}

/// Ce que Git accepterait peut-être ; refuse d'emblée ce qui serait lu comme
/// une option ou une révision spéciale.
fn syntax_ok_pure(name: &str) -> bool {
    !(name.starts_with('-')
        || name == "HEAD"
        || name.contains("@{")
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
        || name.contains(['~', '^', ':', '?', '*', '[', '\\'])
        || name.contains("..")
        || name.ends_with(".lock")
        || name.ends_with('/')
        || name.ends_with('.'))
}

fn local_branch_names(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Vec<String>> {
    let text = git.read(
        repo.work_tree(),
        &args(["for-each-ref", "--format=%(refname:lstrip=2)", "refs/heads"]),
        cancel,
    )?;
    Ok(text
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect())
}

fn ensure_valid_name(
    git: &Git,
    repo: &Repository,
    name: &str,
    ignore: Option<&str>,
    cancel: &CancelToken,
) -> Result<()> {
    match check_branch_name(git, repo, name, ignore, cancel)? {
        Some(problem) => Err(VcsError::InvalidBranchName(problem)),
        None => Ok(()),
    }
}

fn local_exists(
    git: &Git,
    repo: &Repository,
    name: &str,
    cancel: &CancelToken,
) -> Result<Option<String>> {
    check_revision(name)?;
    git.resolve_commit(repo.work_tree(), &format!("refs/heads/{name}"), cancel)
}

fn upstream_of(
    git: &Git,
    repo: &Repository,
    branch: &str,
    cancel: &CancelToken,
) -> Result<Option<String>> {
    let output = git.run(
        repo.work_tree(),
        &args([
            "for-each-ref".to_owned(),
            "--format=%(upstream:short)".to_owned(),
            format!("refs/heads/{branch}"),
        ]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(Some(output.stdout_text().trim().to_owned()).filter(|u| !u.is_empty()))
}

// ------------------------------------------------------------------ créer

/// Créer une branche.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CreateBranchRequest {
    pub name: String,
    /// Point de départ : branche locale ou distante, commit, étiquette ;
    /// `None` : HEAD. Depuis une branche distante, la nouvelle la suit
    /// (`--track`).
    pub start: Option<String>,
    /// Basculer sur la nouvelle branche.
    pub switch: bool,
    /// Si la bascule doit changer de commit et que des changements locaux
    /// gênent.
    pub local_changes: LocalChanges,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBranchOutcome {
    pub name: String,
    /// Commit de la branche ; `None` dans un dépôt sans commit (branche à
    /// naître renommée).
    pub commit: Option<String>,
    /// Amont posé (`origin/x`) si créée depuis une branche distante.
    pub upstream: Option<String>,
    /// La bascule, si demandée.
    pub switched: Option<SwitchOutcome>,
}

/// Crée une branche, et y bascule si demandé. Nom vérifié d'abord
/// ([`VcsError::InvalidBranchName`]). Dépôt sans commit : seule la création
/// avec bascule depuis HEAD a un sens (la branche à naître est renommée, par
/// `git symbolic-ref` : l'index n'est pas touché). Si la bascule échoue, la
/// branche créée est retirée : rien n'a changé. **Bloquant.**
pub fn create_branch(
    git: &Git,
    repo: &Repository,
    request: &CreateBranchRequest,
    cancel: &CancelToken,
) -> Result<CreateBranchOutcome> {
    let name = request.name.as_str();
    ensure_valid_name(git, repo, name, None, cancel)?;
    if let Some(start) = &request.start {
        check_revision(start)?;
    }

    if !has_head(git, repo, cancel)? {
        if request.start.is_some() || !request.switch {
            return Err(VcsError::NoCommits);
        }
        git.run_ok(
            repo.work_tree(),
            &args([
                "symbolic-ref".to_owned(),
                "HEAD".to_owned(),
                format!("refs/heads/{name}"),
            ]),
            Access::Write,
            None,
            cancel,
        )?;
        return Ok(CreateBranchOutcome {
            name: name.to_owned(),
            commit: None,
            upstream: None,
            switched: None,
        });
    }

    let start_text = request.start.clone().unwrap_or_else(|| "HEAD".to_owned());
    let commit = git
        .resolve_commit(repo.work_tree(), &start_text, cancel)?
        .ok_or_else(|| VcsError::InvalidRevision(start_text.clone()))?;
    let track = match &request.start {
        Some(start) => is_remote_branch(git, repo, start, cancel)?,
        None => false,
    };
    let head = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;

    let switched = if request.switch && head.as_deref() == Some(commit.as_str()) {
        // Même commit : rien ne peut être écrasé, les changements suivent.
        let mut command = args(["switch", "-c", name]);
        if track {
            command.push("--track".into());
        }
        if let Some(start) = &request.start {
            command.push(start.into());
        }
        let output = git.run(repo.work_tree(), &command, Access::Write, None, cancel)?;
        let text = format!("{}{}", output.stdout_text(), output.stderr_text());
        let landed = current_branch_name(repo.git_dir()).as_deref() == Some(name);
        if !output.status.success() && !landed {
            return Err(failure(&command, &output));
        }
        Some(SwitchOutcome {
            branch: Some(name.to_owned()),
            head: commit.clone(),
            created: true,
            stash: None,
            hook_output: (!output.status.success()).then(|| text.clone()),
            behind_upstream: None,
            output: text,
        })
    } else {
        let mut command = args(["branch"]);
        if track {
            command.push("--track".into());
        }
        command.push(name.into());
        command.push(start_text.as_str().into());
        git.run_ok(repo.work_tree(), &command, Access::Write, None, cancel)?;
        if request.switch {
            let request_switch = SwitchRequest {
                target: SwitchTarget::Branch(name.to_owned()),
                local_changes: request.local_changes,
            };
            match switch_branch(git, repo, &request_switch, cancel) {
                Ok(mut outcome) => {
                    outcome.created = true;
                    Some(outcome)
                }
                Err(error) => {
                    // Défaire la création : la branche vient d'être faite par nous.
                    let _ = git.run(
                        repo.work_tree(),
                        &args(["branch", "-D", name]),
                        Access::Write,
                        None,
                        &CancelToken::new(),
                    );
                    return Err(error);
                }
            }
        } else {
            None
        }
    };
    Ok(CreateBranchOutcome {
        name: name.to_owned(),
        commit: Some(commit),
        upstream: if track {
            upstream_of(git, repo, name, cancel)?
        } else {
            None
        },
        switched,
    })
}

fn is_remote_branch(
    git: &Git,
    repo: &Repository,
    revision: &str,
    cancel: &CancelToken,
) -> Result<bool> {
    let output = git.run(
        repo.work_tree(),
        &args(["rev-parse", "--symbolic-full-name", revision]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(output.status.success() && output.stdout_text().trim().starts_with("refs/remotes/"))
}

// ------------------------------------------------------------------ renommer, supprimer

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameBranchOutcome {
    pub old: String,
    pub new: String,
    /// L'amont de la branche : la branche distante **garde son nom** (à dire).
    pub upstream: Option<String>,
}

/// `git branch -m old new`, nom vérifié d'abord. **Bloquant.**
pub fn rename_branch(
    git: &Git,
    repo: &Repository,
    old: &str,
    new: &str,
    cancel: &CancelToken,
) -> Result<RenameBranchOutcome> {
    if local_exists(git, repo, old, cancel)?.is_none()
        && current_branch_name(repo.git_dir()).as_deref() != Some(old)
    {
        return Err(VcsError::BranchNotFound(old.to_owned()));
    }
    ensure_valid_name(git, repo, new, Some(old), cancel)?;
    let upstream = upstream_of(git, repo, old, cancel)?;
    git.run_ok(
        repo.work_tree(),
        &args(["branch", "-m", old, new]),
        Access::Write,
        None,
        cancel,
    )?;
    Ok(RenameBranchOutcome {
        old: old.to_owned(),
        new: new.to_owned(),
        upstream,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteBranchOutcome {
    pub name: String,
    /// Sommet au moment de la suppression : pour « Restaurer »
    /// ([`restore_branch`]).
    pub tip: String,
    /// Son amont (case « Supprimer aussi origin/X »).
    pub upstream: Option<String>,
}

/// `git branch -d` : refuse une branche non fusionnée
/// ([`VcsError::BranchNotMerged`], avec le nombre de commits et leurs sujets),
/// et la branche courante ([`VcsError::CurrentBranch`]). **Bloquant.**
pub fn delete_branch(
    git: &Git,
    repo: &Repository,
    name: &str,
    cancel: &CancelToken,
) -> Result<DeleteBranchOutcome> {
    delete(git, repo, name, false, cancel)
}

/// **Destructif** : `git branch -D`, après la confirmation de l'appelant
/// (« n commits ne seront plus sur aucune branche »). Récupérable par
/// [`restore_branch`] avec le `tip` rendu. **Bloquant.**
pub fn force_delete_branch(
    git: &Git,
    repo: &Repository,
    name: &str,
    cancel: &CancelToken,
) -> Result<DeleteBranchOutcome> {
    delete(git, repo, name, true, cancel)
}

fn delete(
    git: &Git,
    repo: &Repository,
    name: &str,
    force: bool,
    cancel: &CancelToken,
) -> Result<DeleteBranchOutcome> {
    let tip = local_exists(git, repo, name, cancel)?
        .ok_or_else(|| VcsError::BranchNotFound(name.to_owned()))?;
    if current_branch_name(repo.git_dir()).as_deref() == Some(name) {
        return Err(VcsError::CurrentBranch(name.to_owned()));
    }
    let upstream = upstream_of(git, repo, name, cancel)?;
    let command = args(["branch", if force { "-D" } else { "-d" }, name]);
    let output = git.run(repo.work_tree(), &command, Access::Write, None, cancel)?;
    if !output.status.success() {
        if !force && output.stderr_text().contains("not fully merged") {
            let (commits, subjects) = only_reachable_from(
                git,
                repo,
                &[format!("refs/heads/{name}")],
                Some(name),
                5,
                cancel,
            )?;
            return Err(VcsError::BranchNotMerged {
                branch: name.to_owned(),
                tip,
                commits,
                subjects,
            });
        }
        return Err(failure(&command, &output));
    }
    Ok(DeleteBranchOutcome {
        name: name.to_owned(),
        tip,
        upstream,
    })
}

/// Commits atteignables depuis `positive` mais depuis aucune autre référence
/// (branches locales sauf `except`, distantes, étiquettes) : leur nombre et les
/// sujets des `subjects` premiers. Les révisions passent par l'entrée standard
/// (`--stdin`) : pas de limite de longueur de ligne de commande avec des
/// milliers de branches. **Bloquant.**
pub(crate) fn only_reachable_from(
    git: &Git,
    repo: &Repository,
    positive: &[String],
    except: Option<&str>,
    subjects: usize,
    cancel: &CancelToken,
) -> Result<(u32, Vec<String>)> {
    let mut input = String::new();
    for revision in positive {
        check_revision(revision)?;
        input.push_str(revision);
        input.push('\n');
    }
    input.push_str("--not\n--remotes\n--tags\n");
    for branch in local_branch_names(git, repo, cancel)? {
        if Some(branch.as_str()) != except {
            input.push_str(&format!("refs/heads/{branch}\n"));
        }
    }
    let count_text = git
        .run_ok(
            repo.work_tree(),
            &args(["rev-list", "--count", "--stdin"]),
            Access::Read,
            Some(input.clone().into_bytes()),
            cancel,
        )?
        .stdout_text();
    let count = count_text
        .trim()
        .parse()
        .map_err(|_| VcsError::UnexpectedOutput(format!("rev-list --count : {count_text:?}")))?;
    let mut found = Vec::new();
    if subjects > 0 && count > 0 {
        let text = git
            .run_ok(
                repo.work_tree(),
                &args([
                    "log".to_owned(),
                    "--format=%s".to_owned(),
                    format!("--max-count={subjects}"),
                    "--stdin".to_owned(),
                ]),
                Access::Read,
                Some(input.into_bytes()),
                cancel,
            )?
            .stdout_text();
        found = text.lines().map(str::to_owned).collect();
    }
    Ok((count, found))
}

/// Recrée une branche supprimée sur le même commit (« Restaurer »). Refuse si
/// le nom est repris entre-temps. **Bloquant.**
pub fn restore_branch(
    git: &Git,
    repo: &Repository,
    name: &str,
    tip: &str,
    cancel: &CancelToken,
) -> Result<()> {
    check_revision(tip)?;
    ensure_valid_name(git, repo, name, None, cancel)?;
    git.run_ok(
        repo.work_tree(),
        &args(["branch", "--no-track", name, tip]),
        Access::Write,
        None,
        cancel,
    )?;
    Ok(())
}

// ------------------------------------------------------------------ basculer

/// Où basculer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SwitchTarget {
    /// Branche locale (`feature/login`).
    Branch(String),
    /// Branche distante (`origin/feature/login`) : bascule sur la locale du même
    /// nom si elle existe, sinon la crée en la faisant suivre la distante.
    Remote(String),
    /// Commit, étiquette ou toute révision : HEAD détachée.
    Detach(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchRequest {
    pub target: SwitchTarget,
    pub local_changes: LocalChanges,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchOutcome {
    /// Branche courante après la bascule (`None` : HEAD détachée).
    pub branch: Option<String>,
    /// Commit de HEAD après la bascule.
    pub head: String,
    /// Une branche locale a été créée (depuis une distante, ou par
    /// [`create_branch`]).
    pub created: bool,
    /// La mise de côté, si [`LocalChanges::Stash`] avait quelque chose à
    /// ranger.
    pub stash: Option<AutoStash>,
    /// Bascule faite mais un crochet (`post-checkout`) a échoué : sa sortie.
    pub hook_output: Option<String>,
    /// La locale existait déjà et a du retard sur son amont : proposer de la
    /// mettre à jour.
    pub behind_upstream: Option<u32>,
    pub output: String,
}

/// Bascule. Refusée pendant une fusion ou un rebase
/// ([`VcsError::OperationInProgress`]). Avec [`LocalChanges::Keep`], si Git
/// refuse parce que des changements seraient écrasés, rend
/// [`VcsError::WouldOverwrite`] avec les fichiers (rien n'a changé). Avec
/// [`LocalChanges::Stash`] : mise de côté « avant bascule », bascule,
/// restauration ; une restauration en conflit garde l'entrée
/// ([`crate::StashRestore::Conflicts`]). **Bloquant.**
pub fn switch_branch(
    git: &Git,
    repo: &Repository,
    request: &SwitchRequest,
    cancel: &CancelToken,
) -> Result<SwitchOutcome> {
    if let Some(operation) = repo.operation() {
        return Err(VcsError::OperationInProgress(operation.kind));
    }
    let from = current_branch_name(repo.git_dir())
        .or_else(|| {
            git.resolve_commit(repo.work_tree(), "HEAD", cancel)
                .ok()
                .flatten()
                .map(|id| id[..id.len().min(7)].to_owned())
        })
        .unwrap_or_default();

    let mut created = false;
    let mut existed_with_upstream = false;
    let (command, expect) = match &request.target {
        SwitchTarget::Branch(name) => {
            local_exists(git, repo, name, cancel)?
                .ok_or_else(|| VcsError::BranchNotFound(name.clone()))?;
            existed_with_upstream = true;
            (
                args(["switch", "--no-guess", name]),
                Expected::Branch(name.clone()),
            )
        }
        SwitchTarget::Remote(remote_name) => {
            check_revision(remote_name)?;
            git.resolve_commit(
                repo.work_tree(),
                &format!("refs/remotes/{remote_name}"),
                cancel,
            )?
            .ok_or_else(|| VcsError::BranchNotFound(remote_name.clone()))?;
            let remotes = list_remotes(git, repo, cancel)?;
            let (_, short) = split_remote(remote_name, &remotes);
            let short = short.to_owned();
            if local_exists(git, repo, &short, cancel)?.is_some() {
                existed_with_upstream = true;
                (
                    args(["switch", "--no-guess", &short]),
                    Expected::Branch(short),
                )
            } else {
                ensure_valid_name(git, repo, &short, None, cancel)?;
                created = true;
                (
                    args([
                        "switch".to_owned(),
                        "-c".to_owned(),
                        short.clone(),
                        "--track".to_owned(),
                        remote_name.clone(),
                    ]),
                    Expected::Branch(short),
                )
            }
        }
        SwitchTarget::Detach(revision) => {
            check_revision(revision)?;
            let commit = git
                .resolve_commit(repo.work_tree(), revision, cancel)?
                .ok_or_else(|| VcsError::InvalidRevision(revision.clone()))?;
            (
                args(["switch", "--detach", &commit]),
                Expected::Detached(commit),
            )
        }
    };
    let to = match &expect {
        Expected::Branch(name) => name.clone(),
        Expected::Detached(commit) => commit[..commit.len().min(7)].to_owned(),
    };

    let ((output, hook_output), stash) = with_local_changes(
        git,
        repo,
        request.local_changes,
        || StashReason::Switch {
            from: from.clone(),
            to: to.clone(),
        },
        cancel,
        || run_switch(git, repo, &command, &expect, cancel).map(|value| (value, true)),
    )?;

    let head = git
        .resolve_commit(repo.work_tree(), "HEAD", cancel)?
        .unwrap_or_default();
    let behind_upstream = if existed_with_upstream {
        upstream_behind(git, repo, cancel)?.filter(|&n| n > 0)
    } else {
        None
    };
    Ok(SwitchOutcome {
        branch: current_branch_name(repo.git_dir()),
        head,
        created,
        stash,
        hook_output,
        behind_upstream,
        output,
    })
}

enum Expected {
    Branch(String),
    Detached(String),
}

/// Lance la bascule ; rend (sortie, sortie du crochet en échec).
fn run_switch(
    git: &Git,
    repo: &Repository,
    command: &[OsString],
    expect: &Expected,
    cancel: &CancelToken,
) -> Result<(String, Option<String>)> {
    let output = git.run(repo.work_tree(), command, Access::Write, None, cancel)?;
    let text = format!("{}{}", output.stdout_text(), output.stderr_text());
    if output.status.success() {
        return Ok((text, None));
    }
    // La bascule a eu lieu mais un crochet a échoué après coup.
    let landed = match expect {
        Expected::Branch(name) => {
            current_branch_name(repo.git_dir()).as_deref() == Some(name.as_str())
        }
        Expected::Detached(commit) => {
            current_branch_name(repo.git_dir()).is_none()
                && git
                    .resolve_commit(repo.work_tree(), "HEAD", cancel)?
                    .as_deref()
                    == Some(commit.as_str())
        }
    };
    if landed && !text.contains("would be overwritten") {
        return Ok((text.clone(), Some(text)));
    }
    if let Some(conflict) = parse_overwrite(&output.stderr_text(), repo) {
        return Err(VcsError::WouldOverwrite(conflict));
    }
    Err(failure(command, &output))
}

fn upstream_behind(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Option<u32>> {
    let output = git.run(
        repo.work_tree(),
        &args(["rev-list", "--count", "HEAD..@{upstream}"]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(output
        .status
        .success()
        .then(|| output.stdout_text().trim().parse().ok())
        .flatten())
}

/// **Destructif** : « Forcer la bascule ». Jette les changements **des seuls
/// fichiers nommés** par le refus (`discard`, rendu par
/// [`VcsError::WouldOverwrite`] et confirmé par l'utilisatrice) : les suivis
/// reviennent à HEAD, les non suivis qui gênent sont supprimés du disque ; les
/// autres changements locaux suivent la bascule. L'appelant prend sa copie de
/// sécurité avant (`decisions-v1.md` n° 3). **Bloquant.**
pub fn switch_discarding_changes(
    git: &Git,
    repo: &Repository,
    target: &SwitchTarget,
    discard: &OverwriteConflict,
    cancel: &CancelToken,
) -> Result<SwitchOutcome> {
    if let Some(operation) = repo.operation() {
        return Err(VcsError::OperationInProgress(operation.kind));
    }
    if !discard.tracked.is_empty() {
        let mut command = args(["restore", "--source=HEAD", "--staged", "--worktree", "--"]);
        for path in &discard.tracked {
            command.push(repo.to_repo_path(path)?.into());
        }
        git.run_ok(repo.work_tree(), &command, Access::Write, None, cancel)?;
    }
    for path in &discard.untracked {
        let repo_path = repo.to_repo_path(path)?;
        let absolute: PathBuf = repo.to_absolute(&repo_path);
        match std::fs::remove_file(&absolute) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(julie_core::Error::io(&absolute, source).into()),
        }
    }
    switch_branch(
        git,
        repo,
        &SwitchRequest {
            target: target.clone(),
            local_changes: LocalChanges::Keep,
        },
        cancel,
    )
}

/// HEAD détachée : nombre de commits qui ne sont sur aucune branche, aucune
/// branche distante ni étiquette (avertissement avant de la quitter). 0 sur une
/// branche. **Bloquant.**
pub fn commits_only_on_detached_head(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<u32> {
    if current_branch_name(repo.git_dir()).is_some() || !has_head(git, repo, cancel)? {
        return Ok(0);
    }
    Ok(only_reachable_from(git, repo, &["HEAD".to_owned()], None, 0, cancel)?.0)
}

/// `ancestor` est-il contenu dans `descendant` (cherry-pick déjà présent,
/// branche fusionnée…) ? **Bloquant.**
pub fn is_ancestor(
    git: &Git,
    repo: &Repository,
    ancestor: &str,
    descendant: &str,
    cancel: &CancelToken,
) -> Result<bool> {
    check_revision(ancestor)?;
    check_revision(descendant)?;
    let output = git.run(
        repo.work_tree(),
        &args(["merge-base", "--is-ancestor", ancestor, descendant]),
        Access::Read,
        None,
        cancel,
    )?;
    match output.status.code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failure(&args(["merge-base", "--is-ancestor"]), &output)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn for_each_ref_output_is_split_into_local_and_remote() {
        let rows: [[&str; 9]; 5] = [
            [
                "refs/heads/feature/login",
                "aaa",
                "origin/feature/login",
                "gone",
                " ",
                "",
                "1700000000",
                "Ana",
                "Connexion",
            ],
            [
                "refs/heads/main",
                "bbb",
                "origin/main",
                "ahead 1, behind 2",
                "*",
                "",
                "1700000001",
                "Bob",
                "Accueil",
            ],
            [
                "refs/heads/seule",
                "ccc",
                "",
                "",
                " ",
                "",
                "1700000002",
                "Bob",
                "x",
            ],
            [
                "refs/remotes/origin/HEAD",
                "bbb",
                "",
                "",
                " ",
                "refs/remotes/origin/main",
                "0",
                "",
                "",
            ],
            [
                "refs/remotes/team/a/feature/x",
                "ddd",
                "",
                "",
                " ",
                "",
                "1700000003",
                "Cid",
                "y",
            ],
        ];
        let text: String = rows.iter().map(|row| row.join("\0") + "\n").collect();
        let text = text.as_str();
        let list = parse_branches(text, &["origin".into(), "team/a".into()]).unwrap();
        assert_eq!(list.local.len(), 3);
        let main = list.find_local("main").unwrap();
        assert!(main.current);
        assert_eq!(
            main.tracking,
            Some(Tracking {
                upstream: "origin/main".into(),
                ahead: 1,
                behind: 2,
                gone: false
            })
        );
        assert!(
            list.find_local("feature/login")
                .unwrap()
                .tracking
                .as_ref()
                .unwrap()
                .gone
        );
        assert_eq!(list.find_local("seule").unwrap().tracking, None);
        assert_eq!(list.remote.len(), 1, "origin/HEAD écartée");
        let remote = &list.remote[0];
        assert_eq!(remote.remote.as_deref(), Some("team/a"));
        assert_eq!(remote.short_name, "feature/x");
        assert_eq!(remote.last_commit.author, "Cid");
    }

    #[test]
    fn recent_branches_come_from_checkout_entries() {
        let reflog = "checkout: moving from bugfix/crash to main\0\
                      checkout: moving from feature/login to bugfix/crash\0\
                      checkout: moving from main to feature/login\0\
                      checkout: moving from main to 0123abc\0\
                      checkout: moving from x to disparue\0\
                      checkout: moving from main to feature/login\0";
        let recent = parse_recent(reflog, Some("main"), |name| {
            ["main", "bugfix/crash", "feature/login"].contains(&name)
        });
        assert_eq!(recent, vec!["bugfix/crash", "feature/login"]);
    }

    #[test]
    fn branches_are_grouped_by_prefix() {
        let names = [
            "bugfix/crash",
            "feature/login",
            "feature/search",
            "juliestorm-essai/a",
            "main",
        ];
        let tree = branch_tree(&names);
        assert_eq!(
            tree,
            vec![
                BranchNode::Branch {
                    label: "bugfix/crash".into(),
                    index: 0
                },
                BranchNode::Folder {
                    name: "feature".into(),
                    path: "feature".into(),
                    children: vec![
                        BranchNode::Branch {
                            label: "login".into(),
                            index: 1
                        },
                        BranchNode::Branch {
                            label: "search".into(),
                            index: 2
                        },
                    ],
                },
                BranchNode::Branch {
                    label: "juliestorm-essai/a".into(),
                    index: 3
                },
                BranchNode::Branch {
                    label: "main".into(),
                    index: 4
                },
            ]
        );
    }

    #[test]
    fn name_collisions_respect_windows_case() {
        let existing = vec!["main".to_owned(), "feature/login".to_owned()];
        let check = |name: &str| branch_name_collision(name, &existing);
        assert_eq!(
            check("main"),
            Some(BranchNameProblem::AlreadyExists("main".into()))
        );
        assert_eq!(
            check("Main"),
            Some(BranchNameProblem::SameNameDifferentCase("main".into()))
        );
        assert_eq!(
            check("feature"),
            Some(BranchNameProblem::FolderOfExisting("feature/login".into()))
        );
        assert_eq!(
            check("feature/login/x"),
            Some(BranchNameProblem::InsideExisting("feature/login".into()))
        );
        assert_eq!(
            check("Feature/x"),
            Some(BranchNameProblem::FolderCaseMismatch(
                "feature/login".into()
            ))
        );
        assert_eq!(check("feature/x"), None);
        assert_eq!(check("bugfix/y"), None);
    }

    #[test]
    fn spaces_are_suggested_as_dashes() {
        assert_eq!(suggest_branch_name("ma branche"), Some("ma-branche".into()));
        assert_eq!(suggest_branch_name("  a  b "), Some("a-b".into()));
        assert_eq!(suggest_branch_name("ok"), None);
        assert!(!syntax_ok_pure("ma branche"));
        assert!(!syntax_ok_pure("-x"));
        assert!(!syntax_ok_pure("a..b"));
        assert!(!syntax_ok_pure("x.lock"));
        assert!(syntax_ok_pure("feature/é"));
    }

    #[test]
    fn remote_names_may_contain_slashes() {
        let remotes = vec!["origin".to_owned(), "team/a".to_owned()];
        assert_eq!(split_remote("origin/x/y", &remotes), ("origin", "x/y"));
        assert_eq!(split_remote("team/a/z", &remotes), ("team/a", "z"));
        assert_eq!(split_remote("inconnu/z", &remotes), ("inconnu", "z"));
    }
}
