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
use crate::repo::{current_branch_name, OperationKind, Repository};
use crate::stash::{after_operation, AutoStash};
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
    /// Qui est qui : quel étage est **votre travail**, lequel est **ce que vous
    /// intégrez**, avec les noms de branche et de commit. **À utiliser pour les
    /// titres des volets et les boutons** à la place de « nôtre / leur » :
    /// pendant un rebase, l'étage 2 (« ours » de Git) est l'amont et votre
    /// commit est l'étage 3.
    pub labels: ConflictLabels,
}

/// Ce que représente un étage d'un conflit, en mots de métier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SideRole {
    /// La branche courante (fusion, cherry-pick, revert) : votre travail.
    CurrentBranch,
    /// Ce que vous fusionnez dans la branche courante (`git merge X`).
    MergedRevision,
    /// Rebase : la base sur laquelle on rejoue (l'amont, `origin/main`…), avec
    /// les commits déjà rejoués. Ce n'est **pas** votre travail.
    RebaseBase,
    /// Rebase : votre commit en cours de rejeu (`REBASE_HEAD`).
    ReplayedCommit,
    /// Cherry-pick : le commit que vous reprenez.
    PickedCommit,
    /// Revert : l'annulation du commit (son changement inverse).
    RevertedCommit,
    /// Remise d'une mise de côté en conflit (aucune opération en cours) : vos
    /// changements mis de côté.
    StashedChanges,
    /// Remise d'une mise de côté en conflit : l'état de la branche après la
    /// bascule ou la mise à jour.
    UpdatedBranch,
}

/// Un étage nommé.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SideLabel {
    pub role: Option<SideRole>,
    /// Nom à montrer : branche (`main`, `origin/main`, `feature/x`), sinon
    /// `None` (l'interface prend alors le commit court).
    pub name: Option<String>,
    /// Commit complet de ce côté, s'il est connu.
    pub commit: Option<String>,
    /// Sujet de ce commit.
    pub subject: Option<String>,
}

impl SideRole {
    /// Ce côté est-il le travail de l'utilisatrice ?
    pub fn is_yours(self) -> bool {
        matches!(
            self,
            SideRole::CurrentBranch | SideRole::ReplayedCommit | SideRole::StashedChanges
        )
    }
}

/// Les deux étages d'un conflit (2 = `ours` de Git, 3 = `theirs`), nommés.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConflictLabels {
    /// Étage 2 (`git checkout --ours`, [`ConflictSide::Ours`]).
    pub ours: SideLabel,
    /// Étage 3 (`git checkout --theirs`, [`ConflictSide::Theirs`]).
    pub theirs: SideLabel,
}

impl ConflictLabels {
    /// Les rôles seuls, sans noms (fonction pure ; `None` : remise d'une mise
    /// de côté).
    pub fn for_operation(kind: Option<OperationKind>) -> Self {
        let (ours, theirs) = match kind {
            Some(OperationKind::Merge) => (SideRole::CurrentBranch, SideRole::MergedRevision),
            Some(OperationKind::Rebase) => (SideRole::RebaseBase, SideRole::ReplayedCommit),
            Some(OperationKind::CherryPick) => (SideRole::CurrentBranch, SideRole::PickedCommit),
            Some(OperationKind::Revert) => (SideRole::CurrentBranch, SideRole::RevertedCommit),
            None => (SideRole::UpdatedBranch, SideRole::StashedChanges),
        };
        Self {
            ours: SideLabel {
                role: Some(ours),
                ..SideLabel::default()
            },
            theirs: SideLabel {
                role: Some(theirs),
                ..SideLabel::default()
            },
        }
    }

    /// L'étage qui porte **votre travail** : `Ours` pour une fusion, un
    /// cherry-pick, un revert ; **`Theirs` pendant un rebase et pour la remise
    /// d'une mise de côté**. « Garder ma version » = `accept_side(yours())`.
    pub fn yours(&self) -> ConflictSide {
        match self.theirs.role {
            Some(role) if role.is_yours() => ConflictSide::Theirs,
            _ => ConflictSide::Ours,
        }
    }

    /// L'étage de ce que vous intégrez (l'autre).
    pub fn incoming(&self) -> ConflictSide {
        self.yours().other()
    }

    pub fn side(&self, side: ConflictSide) -> &SideLabel {
        match side {
            ConflictSide::Ours => &self.ours,
            ConflictSide::Theirs => &self.theirs,
        }
    }
}

/// Noms et sujets des deux côtés, d'après l'opération en cours. Une lecture
/// qui échoue laisse le nom vide (jamais d'erreur pour un titre).
fn conflict_labels(
    git: &Git,
    repo: &Repository,
    operation: Option<&crate::repo::Operation>,
    cancel: &CancelToken,
) -> ConflictLabels {
    let mut labels = ConflictLabels::for_operation(operation.map(|op| op.kind));
    let head = git
        .resolve_commit(repo.work_tree(), "HEAD", cancel)
        .ok()
        .flatten();
    let current = current_branch_name(repo.git_dir());
    match operation {
        Some(op) if op.kind == OperationKind::Rebase => {
            labels.ours.commit = op.onto.clone().or(head);
            labels.ours.name = labels
                .ours
                .commit
                .as_deref()
                .and_then(|onto| ref_pointing_at(git, repo, onto, cancel));
            labels.theirs.commit = op.their_commit.clone();
            labels.theirs.name = op.rebase_branch.clone();
        }
        Some(op) => {
            labels.ours.commit = head;
            labels.ours.name = current;
            labels.theirs.commit = op.their_commit.clone();
            labels.theirs.name = match op.kind {
                OperationKind::Merge => op.message.as_deref().and_then(merged_name_from_message),
                _ => None,
            };
        }
        None => {
            labels.ours.commit = head;
            labels.ours.name = current;
        }
    }
    for label in [&mut labels.ours, &mut labels.theirs] {
        if let Some(commit) = &label.commit {
            label.subject = git
                .read(
                    repo.work_tree(),
                    &args(["log", "-1", "--format=%s", commit.as_str(), "--"]),
                    cancel,
                )
                .ok()
                .map(|text| text.trim_end().to_owned())
                .filter(|text| !text.is_empty());
        }
    }
    labels
}

/// Nom de ce qu'on fusionne, d'après la première ligne de `MERGE_MSG`
/// (« Merge branch 'feature/x' », « Merge remote-tracking branch
/// 'origin/main' », « Merge commit 'abc' »). Fonction pure.
pub(crate) fn merged_name_from_message(message: &str) -> Option<String> {
    let first = message.lines().next()?;
    let start = first.find('\'')? + 1;
    let end = start + first[start..].find('\'')?;
    Some(first[start..end].to_owned()).filter(|name| !name.is_empty())
}

/// Une branche (distante d'abord, puis locale) dont le sommet est `commit`.
fn ref_pointing_at(
    git: &Git,
    repo: &Repository,
    commit: &str,
    cancel: &CancelToken,
) -> Option<String> {
    let text = git
        .read(
            repo.work_tree(),
            &args([
                "for-each-ref".to_owned(),
                format!("--points-at={commit}"),
                "--format=%(refname)".to_owned(),
                "refs/remotes".to_owned(),
                "refs/heads".to_owned(),
            ]),
            cancel,
        )
        .ok()?;
    // `%(refname:short)` abrège `refs/remotes/origin/HEAD` en `origin` : on
    // écarte les `HEAD` sur le nom complet.
    text.lines()
        .map(str::trim)
        .filter(|name| !name.ends_with("/HEAD"))
        .find_map(|name| {
            name.strip_prefix("refs/remotes/")
                .or_else(|| name.strip_prefix("refs/heads/"))
        })
        .map(str::to_owned)
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
    let operation = repo.operation();
    Ok(ConflictVersions {
        kind,
        labels: conflict_labels(git, repo, operation.as_ref(), cancel),
        operation: operation.map(|operation| operation.kind),
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

/// Côté à garder en entier : un **étage** de Git, pas une personne. Pendant un
/// rebase, `Ours` est l'amont et `Theirs` votre commit : passer par
/// [`ConflictLabels::yours`] pour « Garder ma version ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictSide {
    /// Étage 2 (`--ours`).
    Ours,
    /// Étage 3 (`--theirs`).
    Theirs,
}

impl ConflictSide {
    pub fn other(self) -> Self {
        match self {
            ConflictSide::Ours => ConflictSide::Theirs,
            ConflictSide::Theirs => ConflictSide::Ours,
        }
    }
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
    /// L'opération terminée avait mis de côté des changements locaux (mise à
    /// jour, bascule, fusion… avec [`crate::LocalChanges::Stash`]) : ce qu'il en
    /// est advenu à la remise automatique (`Restored`, `Conflicts`, `Failed`).
    /// `None` : rien n'était mis de côté, ou l'opération n'est pas finie.
    pub stash: Option<AutoStash>,
}

/// Abandon d'une opération ([`abort_operation_restoring`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortOutcome {
    pub kind: OperationKind,
    /// La mise de côté liée à l'opération, remise après l'abandon.
    pub stash: Option<AutoStash>,
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
                stash: None,
            });
        }
        return Err(crate::git::failure(&command, &output));
    }
    let finished = repo.operation().is_none();
    let head = head(git, repo, cancel)?;
    // Opération finie : la mise de côté liée revient (même règle que sans arrêt).
    let stash = if finished {
        after_operation(git, repo)?
    } else {
        None
    };
    Ok(ContinueOutcome {
        kind: operation.kind,
        finished,
        head,
        conflicts: Vec::new(),
        output: text,
        stash,
    })
}

/// Abandonne l'opération en cours (`git merge --abort`, `git rebase --abort`…) :
/// **le dépôt revient à l'état d'avant l'opération, résolutions comprises.**
/// Les changements locaux mis de côté pour l'opération sont ensuite remis
/// (voir [`abort_operation_restoring`], qui dit ce qu'il en est advenu).
/// À n'appeler que sur demande explicite. **Bloquant.**
pub fn abort_operation(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<OperationKind> {
    abort_operation_restoring(git, repo, cancel).map(|outcome| outcome.kind)
}

/// Comme [`abort_operation`], et rend la remise de la mise de côté liée à
/// l'opération (`stash`). **Bloquant.**
pub fn abort_operation_restoring(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<AbortOutcome> {
    let operation = repo.operation().ok_or(VcsError::NoOperationInProgress)?;
    git.run_ok(
        repo.work_tree(),
        &args([operation_command(operation.kind), "--abort"]),
        Access::Write,
        None,
        cancel,
    )?;
    Ok(AbortOutcome {
        kind: operation.kind,
        stash: after_operation(git, repo)?,
    })
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
