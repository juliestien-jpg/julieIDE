//! Opérations sur l'historique : fusion, rebase, cherry-pick, revert,
//! réinitialisation ; passer un commit pendant un rebase. Une opération qui
//! s'arrête sur un conflit n'est pas un échec : elle rend
//! `finished: false` et la liste des conflits, puis on passe par la liste des
//! conflits, le dialogue de fusion et [`crate::continue_operation`] /
//! [`skip_operation`] / [`crate::abort_operation`].

use std::ffi::OsString;
use std::path::PathBuf;

use julie_core::CancelToken;

use crate::conflict::{unmerged_paths, ContinueOutcome};
use crate::content::check_revision;
use crate::error::{OverwriteConflict, Result, VcsError};
use crate::git::{args, failure, Access, Git};
use crate::remote::FetchOutcome;
use crate::repo::{current_branch_name, OperationKind, Repository};
use crate::stash::{
    parse_overwrite, stash_push, with_local_changes, AutoStash, LocalChanges, StashReason,
    StashRestore,
};
use crate::status::{status, FileKind};

/// Résultat d'une fusion, d'un rebase, d'un cherry-pick, d'un revert, d'une
/// mise à jour du projet ou d'un pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationOutcome {
    pub kind: OperationKind,
    /// Terminée ; sinon arrêtée (conflit, crochet…) : `conflicts` et l'état
    /// `Status::operation` disent la suite.
    pub finished: bool,
    /// Rien à faire (« Déjà à jour »).
    pub up_to_date: bool,
    /// Fusion par avance rapide.
    pub fast_forward: bool,
    /// HEAD avant l'opération.
    pub before: Option<String>,
    /// HEAD après.
    pub head: Option<String>,
    /// Mise à jour et pull : commits reçus de l'amont (absents de `before`).
    pub received: u32,
    /// Fichiers qui diffèrent entre `before` et `head` (opération terminée).
    pub changed_files: u32,
    /// Fichiers en conflit si l'opération s'est arrêtée.
    pub conflicts: Vec<PathBuf>,
    pub stash: Option<AutoStash>,
    /// Mise à jour du projet : le fetch qui l'a précédée.
    pub fetch: Option<FetchOutcome>,
    /// Sortie de Git (crochets compris).
    pub output: String,
    /// Cherry-pick / revert : commits passés parce que leur changement était
    /// **déjà** dans la branche (Git s'arrêtait sur un commit vide, sans
    /// conflit). Rien n'est perdu : il n'y avait rien à appliquer.
    pub empty_skipped: Vec<String>,
}

impl OperationOutcome {
    pub(crate) fn new(kind: OperationKind, before: Option<String>) -> Self {
        Self {
            kind,
            finished: true,
            up_to_date: false,
            fast_forward: false,
            head: before.clone(),
            before,
            received: 0,
            changed_files: 0,
            conflicts: Vec::new(),
            stash: None,
            fetch: None,
            output: String::new(),
            empty_skipped: Vec::new(),
        }
    }
}

/// Lance une commande qui peut s'arrêter sur un conflit. Rend (sortie,
/// terminée). Refus « changements locaux écrasés » →
/// [`VcsError::WouldOverwrite`]. **Bloquant.**
pub(crate) fn run_stoppable(
    git: &Git,
    repo: &Repository,
    command: &[OsString],
    cancel: &CancelToken,
) -> Result<(String, bool)> {
    let output = git.run(repo.work_tree(), command, Access::Write, None, cancel)?;
    let text = format!("{}{}", output.stdout_text(), output.stderr_text());
    if output.status.success() {
        return Ok((text, repo.operation().is_none()));
    }
    if repo.operation().is_some() {
        // Arrêtée en cours de route (conflit, commit vide, crochet) : la suite
        // passe par la liste des conflits et continuer / abandonner.
        return Ok((text, false));
    }
    let stderr = output.stderr_text();
    if let Some(conflict) = parse_overwrite(&stderr, repo) {
        return Err(VcsError::WouldOverwrite(conflict));
    }
    if stderr.contains("cannot rebase")
        || stderr.contains("cannot pull with rebase")
        || stderr.contains("Please commit or stash them")
    {
        return Err(VcsError::WouldOverwrite(local_changes_conflict(
            git, repo, &stderr, cancel,
        )?));
    }
    Err(failure(command, &output))
}

/// Rebase refusé parce que l'arbre n'est pas propre : les fichiers suivis
/// modifiés.
fn local_changes_conflict(
    git: &Git,
    repo: &Repository,
    stderr: &str,
    cancel: &CancelToken,
) -> Result<OverwriteConflict> {
    let current = status(git, repo, cancel)?;
    Ok(OverwriteConflict {
        tracked: current
            .files
            .iter()
            .filter(|file| file.kind != FileKind::Untracked)
            .map(|file| file.path.clone())
            .collect(),
        untracked: Vec::new(),
        output: stderr.trim_end().to_owned(),
    })
}

/// Complète un résultat après l'opération : HEAD, conflits, fichiers changés.
pub(crate) fn finish_outcome(
    git: &Git,
    repo: &Repository,
    outcome: &mut OperationOutcome,
    finished: bool,
    output: String,
    cancel: &CancelToken,
) -> Result<()> {
    outcome.finished = finished;
    outcome.output.push_str(&output);
    outcome.head = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    if !finished {
        outcome.conflicts = unmerged_paths(git, repo, cancel)?;
    } else if let (Some(before), Some(head)) = (&outcome.before, &outcome.head) {
        if before != head {
            let text = git.read(
                repo.work_tree(),
                &args(["diff", "--name-only", "-z", before, head, "--"]),
                cancel,
            )?;
            outcome.changed_files = text.split('\0').filter(|p| !p.is_empty()).count() as u32;
        }
    }
    Ok(())
}

pub(crate) fn current_name(git: &Git, repo: &Repository, cancel: &CancelToken) -> String {
    current_branch_name(repo.git_dir())
        .or_else(|| {
            git.resolve_commit(repo.work_tree(), "HEAD", cancel)
                .ok()
                .flatten()
                .map(|id| id[..id.len().min(7)].to_owned())
        })
        .unwrap_or_default()
}

fn ensure_no_operation(repo: &Repository) -> Result<()> {
    match repo.operation() {
        Some(operation) => Err(VcsError::OperationInProgress(operation.kind)),
        None => Ok(()),
    }
}

// ------------------------------------------------------------------ fusion

/// Avance rapide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FastForward {
    /// Avance rapide si possible (défaut de Git).
    #[default]
    Allow,
    /// Toujours un commit de fusion (`--no-ff`).
    Never,
    /// Avance rapide seulement (`--ff-only`).
    Only,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeRequest {
    /// Ce qu'on fusionne dans la branche courante (`feature/login`,
    /// `origin/main`, un commit).
    pub revision: String,
    pub fast_forward: FastForward,
    /// Écraser en un seul changement indexé, sans commit (`--squash`).
    pub squash: bool,
    /// Fusionner sans valider (`--no-commit`) : on conclut ensuite par
    /// [`crate::continue_operation`].
    pub no_commit: bool,
    /// Message du commit de fusion ; `None` : celui de Git.
    pub message: Option<String>,
    pub local_changes: LocalChanges,
}

/// `git merge`. **Bloquant.**
pub fn merge(
    git: &Git,
    repo: &Repository,
    request: &MergeRequest,
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    ensure_no_operation(repo)?;
    check_revision(&request.revision)?;
    let incoming = git
        .resolve_commit(repo.work_tree(), &request.revision, cancel)?
        .ok_or_else(|| VcsError::InvalidRevision(request.revision.clone()))?;
    let before = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    let mut outcome = OperationOutcome::new(OperationKind::Merge, before.clone());
    if let Some(before) = &before {
        if crate::branches::is_ancestor(git, repo, &incoming, before, cancel)? {
            outcome.up_to_date = true;
            return Ok(outcome);
        }
    }
    let mut command = args(["merge"]);
    command.push(
        match request.fast_forward {
            FastForward::Allow => "--ff",
            FastForward::Never => "--no-ff",
            FastForward::Only => "--ff-only",
        }
        .into(),
    );
    if request.squash {
        command.push("--squash".into());
    }
    if request.no_commit {
        command.push("--no-commit".into());
    }
    match &request.message {
        Some(message) if !message.trim().is_empty() => {
            command.push("-m".into());
            command.push(message.into());
        }
        _ => command.push("--no-edit".into()),
    }
    command.push(request.revision.as_str().into());

    let branch = current_name(git, repo, cancel);
    let ((text, finished), stash) = with_local_changes(
        git,
        repo,
        request.local_changes,
        || StashReason::Merge {
            branch,
            revision: request.revision.clone(),
        },
        cancel,
        || {
            run_stoppable(git, repo, &command, cancel)
                .map(|(text, finished)| ((text, finished), finished))
        },
    )?;
    outcome.stash = stash;
    finish_outcome(git, repo, &mut outcome, finished, text, cancel)?;
    outcome.fast_forward = finished && outcome.head.as_deref() == Some(incoming.as_str());
    Ok(outcome)
}

// ------------------------------------------------------------------ rebase

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RebaseRequest {
    /// Sur quoi rejouer (`main`, `origin/main`).
    pub onto: String,
    /// La branche à rejouer (`git rebase onto branche` : « Basculer et rebaser
    /// sur C ») ; `None` : la branche courante.
    pub branch: Option<String>,
    pub local_changes: LocalChanges,
}

/// `git rebase onto [branche]`. Rebase interactif : hors périmètre.
/// **Bloquant.**
pub fn rebase(
    git: &Git,
    repo: &Repository,
    request: &RebaseRequest,
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    ensure_no_operation(repo)?;
    check_revision(&request.onto)?;
    let mut command = args(["rebase", request.onto.as_str()]);
    let branch_name = match &request.branch {
        Some(branch) => {
            check_revision(branch)?;
            command.push(branch.into());
            branch.clone()
        }
        None => current_name(git, repo, cancel),
    };
    let before = git.resolve_commit(
        repo.work_tree(),
        request.branch.as_deref().unwrap_or("HEAD"),
        cancel,
    )?;
    let mut outcome = OperationOutcome::new(OperationKind::Rebase, before);
    let ((text, finished), stash) = with_local_changes(
        git,
        repo,
        request.local_changes,
        || StashReason::Rebase {
            branch: branch_name,
            onto: request.onto.clone(),
        },
        cancel,
        || {
            run_stoppable(git, repo, &command, cancel)
                .map(|(text, finished)| ((text, finished), finished))
        },
    )?;
    outcome.stash = stash;
    outcome.up_to_date = finished && text.contains("is up to date");
    finish_outcome(git, repo, &mut outcome, finished, text, cancel)?;
    Ok(outcome)
}

// ------------------------------------------------------------------ cherry-pick, revert

fn ensure_not_merges(
    git: &Git,
    repo: &Repository,
    commits: &[String],
    cancel: &CancelToken,
) -> Result<()> {
    if commits.is_empty() {
        return Err(VcsError::InvalidRequest("aucun commit".into()));
    }
    for commit in commits {
        check_revision(commit)?;
        let text = git.read(
            repo.work_tree(),
            &args(["rev-list", "--parents", "-n", "1", commit.as_str(), "--"]),
            cancel,
        )?;
        if text.split_whitespace().count() > 2 {
            return Err(VcsError::MergeCommitNotSupported(commit.clone()));
        }
    }
    Ok(())
}

/// Un commit est-il déjà dans la branche courante ?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommitPresence {
    /// Absent : le cherry-pick a un sens.
    Absent,
    /// Le commit lui-même est dans l'historique de HEAD (« déjà sur la branche
    /// courante »).
    Contained,
    /// Un autre commit de la branche apporte exactement le même changement
    /// (`git cherry` : même identifiant de patch).
    SameChange,
}

impl CommitPresence {
    pub fn is_present(self) -> bool {
        self != CommitPresence::Absent
    }
}

/// Pour griser « Cherry-pick » : chaque commit (tel que donné) et sa présence
/// dans la branche courante. Dépôt sans commit : tous absents. **Bloquant.**
pub fn commits_presence(
    git: &Git,
    repo: &Repository,
    commits: &[String],
    cancel: &CancelToken,
) -> Result<Vec<(String, CommitPresence)>> {
    let head = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    let mut result = Vec::new();
    for commit in commits {
        check_revision(commit)?;
        let full = git
            .resolve_commit(repo.work_tree(), commit, cancel)?
            .ok_or_else(|| VcsError::InvalidRevision(commit.clone()))?;
        let Some(head) = &head else {
            result.push((commit.clone(), CommitPresence::Absent));
            continue;
        };
        if crate::branches::is_ancestor(git, repo, &full, head, cancel)? {
            result.push((commit.clone(), CommitPresence::Contained));
            continue;
        }
        let parent = format!("{full}^");
        let mut command = args(["cherry", head.as_str(), full.as_str()]);
        if git
            .resolve_commit(repo.work_tree(), &parent, cancel)?
            .is_some()
        {
            command.push(parent.into());
        }
        let text = git.read(repo.work_tree(), &command, cancel)?;
        let same = text.lines().any(|line| line.trim() == format!("- {full}"));
        result.push((
            commit.clone(),
            if same {
                CommitPresence::SameChange
            } else {
                CommitPresence::Absent
            },
        ));
    }
    Ok(result)
}

/// `git cherry-pick <commits>` dans l'ordre donné (l'appelant les donne du
/// plus ancien au plus récent). Commit de fusion refusé
/// ([`VcsError::MergeCommitNotSupported`]) ; commit **déjà** dans la branche
/// (lui-même ou le même changement) refusé avant toute commande
/// ([`VcsError::AlreadyApplied`], rien n'a changé). Un commit devenu vide en
/// route est passé ([`OperationOutcome::empty_skipped`]) : jamais d'état
/// « cherry-pick en cours » sans rien à résoudre. **Bloquant.**
pub fn cherry_pick(
    git: &Git,
    repo: &Repository,
    commits: &[String],
    local_changes: LocalChanges,
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    ensure_no_operation(repo)?;
    ensure_not_merges(git, repo, commits, cancel)?;
    let present: Vec<String> = commits_presence(git, repo, commits, cancel)?
        .into_iter()
        .filter(|(_, presence)| presence.is_present())
        .map(|(commit, _)| commit)
        .collect();
    if !present.is_empty() {
        return Err(VcsError::AlreadyApplied { commits: present });
    }
    let mut command = args(["cherry-pick"]);
    command.extend(commits.iter().map(OsString::from));
    pick_like(
        git,
        repo,
        OperationKind::CherryPick,
        command,
        local_changes,
        cancel,
    )
}

/// `git revert --no-edit <commits>` : un commit inverse par commit. Commit de
/// fusion refusé. **Bloquant.**
pub fn revert_commits(
    git: &Git,
    repo: &Repository,
    commits: &[String],
    local_changes: LocalChanges,
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    ensure_no_operation(repo)?;
    ensure_not_merges(git, repo, commits, cancel)?;
    let mut command = args(["revert", "--no-edit"]);
    command.extend(commits.iter().map(OsString::from));
    pick_like(
        git,
        repo,
        OperationKind::Revert,
        command,
        local_changes,
        cancel,
    )
}

fn pick_like(
    git: &Git,
    repo: &Repository,
    kind: OperationKind,
    command: Vec<OsString>,
    local_changes: LocalChanges,
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    let before = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    let mut outcome = OperationOutcome::new(kind, before);
    let branch = current_name(git, repo, cancel);
    let mut skipped = Vec::new();
    let ((text, finished), stash) = with_local_changes(
        git,
        repo,
        local_changes,
        || match kind {
            OperationKind::Revert => StashReason::Revert { branch },
            _ => StashReason::CherryPick { branch },
        },
        cancel,
        || {
            let (mut text, mut finished) = run_stoppable(git, repo, &command, cancel)?;
            while !finished {
                let Some(commit) = empty_stop(git, repo, kind, cancel)? else {
                    break;
                };
                let skip = args([
                    if kind == OperationKind::Revert {
                        "revert"
                    } else {
                        "cherry-pick"
                    },
                    "--skip",
                ]);
                let (more, done) = run_stoppable(git, repo, &skip, cancel)?;
                text.push_str(&more);
                skipped.push(commit);
                finished = done;
            }
            Ok(((text, finished), finished))
        },
    )?;
    outcome.stash = stash;
    outcome.empty_skipped = skipped;
    finish_outcome(git, repo, &mut outcome, finished, text, cancel)?;
    Ok(outcome)
}

/// L'opération `kind` est arrêtée sur un commit **vide** (aucun conflit,
/// index identique à HEAD) : son commit. Sinon `None`.
fn empty_stop(
    git: &Git,
    repo: &Repository,
    kind: OperationKind,
    cancel: &CancelToken,
) -> Result<Option<String>> {
    let Some(operation) = repo.operation().filter(|op| op.kind == kind) else {
        return Ok(None);
    };
    if !unmerged_paths(git, repo, cancel)?.is_empty() {
        return Ok(None);
    }
    let output = git.run(
        repo.work_tree(),
        &args(["diff", "--cached", "--quiet", "HEAD", "--"]),
        Access::Read,
        None,
        cancel,
    )?;
    if output.status.code != Some(0) {
        return Ok(None);
    }
    Ok(Some(operation.their_commit.unwrap_or_default()))
}

/// Passe le commit en cours d'un rebase, d'un cherry-pick ou d'un revert
/// (`git <op> --skip`) : **ses changements sont abandonnés** (l'appelant a
/// confirmé en nommant le commit). Peut s'arrêter sur le conflit suivant.
/// **Bloquant.**
pub fn skip_operation(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<ContinueOutcome> {
    let operation = repo.operation().ok_or(VcsError::NoOperationInProgress)?;
    let name = match operation.kind {
        OperationKind::Rebase => "rebase",
        OperationKind::CherryPick => "cherry-pick",
        OperationKind::Revert => "revert",
        OperationKind::Merge => {
            return Err(VcsError::InvalidRequest(
                "une fusion ne se passe pas : la conclure ou l'abandonner".into(),
            ))
        }
    };
    let command = args([name, "--skip"]);
    let (text, finished) = run_stoppable(git, repo, &command, cancel)?;
    Ok(ContinueOutcome {
        kind: operation.kind,
        finished,
        head: git.resolve_commit(repo.work_tree(), "HEAD", cancel)?,
        conflicts: if finished {
            Vec::new()
        } else {
            unmerged_paths(git, repo, cancel)?
        },
        output: text,
        stash: if finished {
            crate::stash::after_operation(git, repo)?
        } else {
            None
        },
    })
}

// ------------------------------------------------------------------ réinitialiser

/// Modes non destructifs de `git reset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetMode {
    /// `--soft` : tout reste indexé.
    Soft,
    /// `--mixed` : les changements restent dans les fichiers, non indexés.
    Mixed,
}

/// Ce que dira le dialogue de réinitialisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPreview {
    /// Le commit cible.
    pub target: String,
    /// Commits que la branche courante quitte (`cible..HEAD`).
    pub commits_leaving: u32,
    /// Parmi eux, ceux qui ne seront plus sur aucune branche, distante ni
    /// étiquette (« récupérables par le reflog pendant 90 jours »).
    pub unreachable: u32,
    /// Certains étaient déjà poussés : le prochain push devra forcer.
    pub pushed: bool,
    /// Fichiers modifiés localement (seconde confirmation du mode « tout
    /// jeter »).
    pub local_changes: Vec<PathBuf>,
}

/// Prépare le dialogue de réinitialisation. **Bloquant.**
pub fn reset_preview(
    git: &Git,
    repo: &Repository,
    target: &str,
    cancel: &CancelToken,
) -> Result<ResetPreview> {
    check_revision(target)?;
    let commit = git
        .resolve_commit(repo.work_tree(), target, cancel)?
        .ok_or_else(|| VcsError::InvalidRevision(target.to_owned()))?;
    let not_target = format!("^{commit}");
    let commits_leaving = git.count(repo.work_tree(), &["HEAD", &not_target], cancel)?;
    let unpushed = git.count(
        repo.work_tree(),
        &["HEAD", &not_target, "--not", "--remotes"],
        cancel,
    )?;
    let current = current_branch_name(repo.git_dir());
    let (unreachable, _) = crate::branches::only_reachable_from(
        git,
        repo,
        &["HEAD".to_owned(), not_target.clone()],
        current.as_deref(),
        0,
        cancel,
    )?;
    let local_changes = status(git, repo, cancel)?
        .files
        .into_iter()
        .filter(|file| file.kind != FileKind::Untracked)
        .map(|file| file.path)
        .collect();
    Ok(ResetPreview {
        target: commit,
        commits_leaving,
        unreachable,
        pushed: unpushed < commits_leaving,
        local_changes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetOutcome {
    pub before: Option<String>,
    pub head: String,
    /// `reset_hard` avec mise de côté : l'entrée **laissée** dans
    /// `git stash list` (`StashRestore::Kept`).
    pub stash: Option<AutoStash>,
    pub output: String,
    /// `reset_hard` sans mise de côté : fichiers suivis dont les changements
    /// locaux ont été **réellement jetés** (ceux que l'appelant a dû copier).
    /// Vide pour `--soft` / `--mixed`, et avec mise de côté (tout est dans
    /// `stash`).
    pub discarded: Vec<PathBuf>,
}

/// `git reset --soft|--mixed <cible>` : rien n'est perdu (les fichiers ne
/// changent pas). **Bloquant.**
pub fn reset(
    git: &Git,
    repo: &Repository,
    target: &str,
    mode: ResetMode,
    cancel: &CancelToken,
) -> Result<ResetOutcome> {
    ensure_no_operation(repo)?;
    let flag = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
    };
    run_reset(git, repo, target, flag, None, cancel)
}

/// **Destructif** : `git reset --hard <cible>` — les changements locaux des
/// fichiers suivis sont **jetés**. Avec `stash_first`, ils sont d'abord mis de
/// côté (entrée « avant réinitialisation », laissée dans `git stash list`).
/// L'appelant a confirmé et pris sa copie de sécurité. **Bloquant.**
pub fn reset_hard(
    git: &Git,
    repo: &Repository,
    target: &str,
    stash_first: bool,
    cancel: &CancelToken,
) -> Result<ResetOutcome> {
    ensure_no_operation(repo)?;
    let changed: Vec<PathBuf> = status(git, repo, cancel)?
        .files
        .into_iter()
        .filter(|file| file.kind != FileKind::Untracked)
        .map(|file| file.path)
        .collect();
    let stash = if stash_first {
        let message = StashReason::Reset {
            branch: current_name(git, repo, cancel),
        }
        .message(std::time::SystemTime::now());
        // Non suivis compris : ils ne seraient pas touchés par le reset, mais
        // l'utilisatrice a demandé « mettre de côté mes changements ».
        stash_push(git, repo, &message, cancel)?.map(|commit| AutoStash {
            message,
            commit,
            restore: StashRestore::Kept,
            untracked_not_restored: Vec::new(),
        })
    } else {
        None
    };
    let stashed = stash.is_some();
    let mut outcome = run_reset(git, repo, target, "--hard", stash, cancel)?;
    if !stashed {
        outcome.discarded = changed;
    }
    Ok(outcome)
}

fn run_reset(
    git: &Git,
    repo: &Repository,
    target: &str,
    flag: &str,
    stash: Option<AutoStash>,
    cancel: &CancelToken,
) -> Result<ResetOutcome> {
    check_revision(target)?;
    let commit = git
        .resolve_commit(repo.work_tree(), target, cancel)?
        .ok_or_else(|| VcsError::InvalidRevision(target.to_owned()))?;
    let before = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    let output = git.run_ok(
        repo.work_tree(),
        &args(["reset", "-q", flag, commit.as_str()]),
        Access::Write,
        None,
        cancel,
    )?;
    Ok(ResetOutcome {
        before,
        head: commit,
        stash,
        output: format!("{}{}", output.stdout_text(), output.stderr_text()),
        discarded: Vec::new(),
    })
}
