//! Conflits : les trois versions d'un fichier (étages 1, 2, 3), l'enregistrement
//! du résultat, le choix d'un côté entier, la conclusion ou l'abandon de
//! l'opération en cours.
//!
//! La fusion elle-même (calcul du résultat) appartient à `julie-diff` ; ici on
//! fournit les octets et on enregistre ce qu'on nous rend.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use julie_core::CancelToken;

use crate::content::{entry_content, index_entries, read_work_tree, Content, Entry};
use crate::error::{Result, VcsError};
use crate::git::{args, Access, Git};
use crate::repo::{OperationKind, Repository};
use crate::status::ConflictKind;

/// Les versions d'un fichier en conflit, pour le dialogue de fusion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictVersions {
    pub path: PathBuf,
    pub repo_path: String,
    pub kind: ConflictKind,
    /// L'opération en cours (pour les titres : pendant un rebase, « la nôtre »
    /// est la branche sur laquelle on rejoue).
    pub operation: Option<OperationKind>,
    /// Étage 1 ; `Missing` si ajouté des deux côtés.
    pub base: Content,
    /// Étage 2.
    pub ours: Content,
    /// Étage 3.
    pub theirs: Content,
    /// Le fichier sur le disque (avec ses marqueurs `<<<<<<<`), à rendre à
    /// [`resolve_with_content`] pour détecter une modification entre-temps.
    pub work_tree: Content,
}

/// Les étages présents d'un chemin, s'il est en conflit.
fn conflict_stages(
    git: &Git,
    repo: &Repository,
    repo_path: &str,
    cancel: &CancelToken,
) -> Result<[Option<Entry>; 3]> {
    let mut stages: [Option<Entry>; 3] = [None, None, None];
    for (stage, entry) in index_entries(git, repo, repo_path, cancel)? {
        if (1..=3).contains(&stage) {
            stages[usize::from(stage - 1)] = Some(entry);
        }
    }
    Ok(stages)
}

/// Lit base, nôtre, leur et le fichier de travail. **Bloquant.**
pub fn conflict_versions(
    git: &Git,
    repo: &Repository,
    path: &Path,
    cancel: &CancelToken,
) -> Result<ConflictVersions> {
    let repo_path = repo.to_repo_path(path)?;
    let absolute = repo.to_absolute(&repo_path);
    let [base, ours, theirs] = conflict_stages(git, repo, &repo_path, cancel)?;
    let kind = ConflictKind::from_stages(base.is_some(), ours.is_some(), theirs.is_some())
        .ok_or_else(|| VcsError::NotConflicted(absolute.clone()))?;
    Ok(ConflictVersions {
        kind,
        operation: repo.operation().map(|operation| operation.kind),
        base: entry_content(git, repo, &repo_path, base, cancel)?,
        ours: entry_content(git, repo, &repo_path, ours, cancel)?,
        theirs: entry_content(git, repo, &repo_path, theirs, cancel)?,
        work_tree: read_work_tree(&absolute)?,
        path: absolute,
        repo_path,
    })
}

/// Écrit le résultat résolu dans le fichier de travail (écriture atomique) et le
/// marque résolu (`git add`). Si `expected_work_tree` est donné et que le fichier
/// sur le disque n'a plus ces octets, rien n'est écrit
/// ([`VcsError::ChangedOnDisk`]). **Bloquant.**
///
/// Les octets sont écrits tels quels : fins de ligne et encodage sont l'affaire
/// de celui qui a fusionné (`julie-diff`).
pub fn resolve_with_content(
    git: &Git,
    repo: &Repository,
    path: &Path,
    content: &[u8],
    expected_work_tree: Option<&[u8]>,
    cancel: &CancelToken,
) -> Result<()> {
    let repo_path = repo.to_repo_path(path)?;
    let absolute = repo.to_absolute(&repo_path);
    let [base, ours, theirs] = conflict_stages(git, repo, &repo_path, cancel)?;
    if base.is_none() && ours.is_none() && theirs.is_none() {
        return Err(VcsError::NotConflicted(absolute));
    }
    if let Some(expected) = expected_work_tree {
        let current = std::fs::read(&absolute).ok();
        if current.as_deref() != Some(expected) {
            return Err(VcsError::ChangedOnDisk(absolute));
        }
    }
    cancel.check()?;
    julie_core::file::write_atomic(&absolute, content)?;
    mark_resolved(git, repo, &[absolute], cancel)
}

/// Marque résolus des fichiers tels qu'ils sont sur le disque : `git add --all`
/// (un fichier absent du disque est enregistré comme supprimé). **Bloquant.**
pub fn mark_resolved(
    git: &Git,
    repo: &Repository,
    paths: &[PathBuf],
    cancel: &CancelToken,
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = args(["add", "--all", "--"]);
    for path in paths {
        command.push(repo.to_repo_path(path)?.into());
    }
    git.run_ok(repo.work_tree(), &command, Access::Write, None, cancel)?;
    Ok(())
}

/// Côté à garder en entier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictSide {
    /// Étage 2 (« Garder la nôtre »).
    Ours,
    /// Étage 3 (« Prendre la leur »).
    Theirs,
}

/// Ce qui a été fait d'un fichier par [`accept_side`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideResolution {
    pub path: PathBuf,
    /// Le côté choisi n'avait pas le fichier : il a été **supprimé** du disque et
    /// de l'index.
    pub deleted: bool,
}

/// Garde un côté entier pour chaque fichier : `git checkout --ours|--theirs`
/// puis `git add`, ou `git rm` si ce côté avait supprimé le fichier.
/// **Écrase le fichier de travail (marqueurs compris) et peut le supprimer** :
/// l'appelant a demandé confirmation. Tous les chemins doivent être en conflit,
/// sinon rien n'est fait. **Bloquant.**
pub fn accept_side(
    git: &Git,
    repo: &Repository,
    paths: &[PathBuf],
    side: ConflictSide,
    cancel: &CancelToken,
) -> Result<Vec<SideResolution>> {
    let mut keep: Vec<String> = Vec::new();
    let mut delete: Vec<String> = Vec::new();
    let mut results = Vec::new();
    for path in paths {
        let repo_path = repo.to_repo_path(path)?;
        let absolute = repo.to_absolute(&repo_path);
        let [base, ours, theirs] = conflict_stages(git, repo, &repo_path, cancel)?;
        if base.is_none() && ours.is_none() && theirs.is_none() {
            return Err(VcsError::NotConflicted(absolute));
        }
        let chosen = match side {
            ConflictSide::Ours => ours,
            ConflictSide::Theirs => theirs,
        };
        let deleted = chosen.is_none();
        if deleted {
            delete.push(repo_path);
        } else {
            keep.push(repo_path);
        }
        results.push(SideResolution {
            path: absolute,
            deleted,
        });
    }

    if !keep.is_empty() {
        let flag = match side {
            ConflictSide::Ours => "--ours",
            ConflictSide::Theirs => "--theirs",
        };
        let mut checkout = args(["checkout", flag, "--"]);
        checkout.extend(keep.iter().map(OsString::from));
        git.run_ok(repo.work_tree(), &checkout, Access::Write, None, cancel)?;
        let mut add = args(["add", "--"]);
        add.extend(keep.iter().map(OsString::from));
        git.run_ok(repo.work_tree(), &add, Access::Write, None, cancel)?;
    }
    if !delete.is_empty() {
        let mut remove = args(["rm", "--quiet", "--force", "--"]);
        remove.extend(delete.iter().map(OsString::from));
        git.run_ok(repo.work_tree(), &remove, Access::Write, None, cancel)?;
    }
    Ok(results)
}

/// Les chemins encore non fusionnés.
pub fn unmerged_paths(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Vec<PathBuf>> {
    let output = git.run_ok(
        repo.work_tree(),
        &args(["diff", "--name-only", "--diff-filter=U", "-z"]),
        Access::Read,
        None,
        cancel,
    )?;
    let text = String::from_utf8(output.stdout)
        .map_err(|_| VcsError::UnexpectedOutput("diff --name-only : pas de l'UTF-8".into()))?;
    let mut paths: Vec<PathBuf> = text
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(|path| repo.to_absolute(path))
        .collect();
    paths.dedup();
    Ok(paths)
}

/// Suite d'une opération reprise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueOutcome {
    pub kind: OperationKind,
    /// L'opération est terminée (sinon : arrêtée sur un nouveau conflit).
    pub finished: bool,
    /// `HEAD` après la reprise.
    pub head: Option<String>,
    /// Fichiers en conflit si l'opération s'est de nouveau arrêtée.
    pub conflicts: Vec<PathBuf>,
    /// Sortie de Git (crochets compris).
    pub output: String,
}

/// Conclut l'opération en cours une fois tous les conflits résolus. **Bloquant.**
///
/// - Fusion : `git commit -F -` avec `message`, ou `git commit --no-edit` (message
///   de `.git/MERGE_MSG`) si `message` vaut `None`.
/// - Rebase, cherry-pick, revert : `git <op> --continue` ; le message préparé par
///   Git est gardé (`message` est ignoré).
pub fn continue_operation(
    git: &Git,
    repo: &Repository,
    message: Option<&str>,
    cancel: &CancelToken,
) -> Result<ContinueOutcome> {
    let operation = repo.operation().ok_or(VcsError::NoOperationInProgress)?;
    let remaining = unmerged_paths(git, repo, cancel)?;
    if !remaining.is_empty() {
        return Err(VcsError::UnresolvedConflicts(remaining));
    }
    let (command, stdin) = match (operation.kind, message) {
        (OperationKind::Merge, Some(message)) => {
            if message.trim().is_empty() {
                return Err(VcsError::EmptyMessage);
            }
            (
                args(["commit", "-F", "-"]),
                Some(message.as_bytes().to_vec()),
            )
        }
        (OperationKind::Merge, None) => (args(["commit", "--no-edit"]), None),
        (kind, _) => (args([operation_command(kind), "--continue"]), None),
    };
    let output = git.run(repo.work_tree(), &command, Access::Write, stdin, cancel)?;
    let text = format!("{}{}", output.stdout_text(), output.stderr_text());
    if !output.status.success() {
        // Un rebase qui s'arrête sur le commit suivant en conflit n'est pas un échec.
        let conflicts = unmerged_paths(git, repo, cancel)?;
        if repo.operation().is_some() && !conflicts.is_empty() {
            return Ok(ContinueOutcome {
                kind: operation.kind,
                finished: false,
                head: head(git, repo, cancel)?,
                conflicts,
                output: text,
            });
        }
        return Err(crate::git::failure(&command, &output));
    }
    Ok(ContinueOutcome {
        kind: operation.kind,
        finished: repo.operation().is_none(),
        head: head(git, repo, cancel)?,
        conflicts: Vec::new(),
        output: text,
    })
}

/// Abandonne l'opération en cours (`git merge --abort`, `git rebase --abort`…) :
/// **le dépôt revient à l'état d'avant l'opération, résolutions comprises.**
/// À n'appeler que sur demande explicite. **Bloquant.**
pub fn abort_operation(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<OperationKind> {
    let operation = repo.operation().ok_or(VcsError::NoOperationInProgress)?;
    git.run_ok(
        repo.work_tree(),
        &args([operation_command(operation.kind), "--abort"]),
        Access::Write,
        None,
        cancel,
    )?;
    Ok(operation.kind)
}

fn operation_command(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Merge => "merge",
        OperationKind::Rebase => "rebase",
        OperationKind::CherryPick => "cherry-pick",
        OperationKind::Revert => "revert",
    }
}

fn head(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Option<String>> {
    let output = git.run(
        repo.work_tree(),
        &args(["rev-parse", "-q", "--verify", "HEAD"]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(output
        .status
        .success()
        .then(|| output.stdout_text().trim().to_owned()))
}
