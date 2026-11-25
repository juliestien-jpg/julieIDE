//! Le service asynchrone : chaque méthode lance une tâche de fond (`Domain::Vcs`)
//! et rend aussitôt la main ; les résultats arrivent sur le bus en [`VcsEvent`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use julie_core::{CancelToken, Domain, TaskContext, TaskHandle, Tasks};

use crate::branches::{
    check_branch_name, commits_only_on_detached_head, create_branch, delete_branch,
    detached_head_report, force_delete_branch, list_branches, rename_branch, restore_branch,
    restore_deleted_branch, set_upstream, switch_branch_with, switch_discarding_changes_with,
    unset_upstream, upstream_short, BranchList, CreateBranchOutcome, CreateBranchRequest,
    DeleteBranchOutcome, DetachedHeadReport, RenameBranchOutcome, SwitchOptions, SwitchOutcome,
    SwitchRequest, SwitchTarget,
};
use crate::commit::{commit, CommitOutcome, CommitRequest};
use crate::conflict::{
    abort_operation_restoring, accept_side, conflict_versions, continue_operation,
    resolve_with_content, ConflictSide, ConflictVersions, ContinueOutcome, SideResolution,
};
use crate::content::{load_diff, DiffRequest, DiffSides};
use crate::error::{BranchNameProblem, OverwriteConflict};
use crate::error::{Result, VcsError};
use crate::files::{
    add_to_gitignore, discard_changes, move_path, stage, unstage, DiscardOutcome, IgnoreAdded,
    IgnoreRule,
};
use crate::git::{CommandRecord, Git, VcsSettings};
use crate::history::{commit_details, log, CommitDetails, LogPage, LogQuery};
use crate::operations::{
    cherry_pick, commits_presence, merge, rebase, reset, reset_hard, reset_preview, revert_commits,
    skip_operation, CommitPresence, MergeRequest, OperationOutcome, RebaseRequest, ResetMode,
    ResetOutcome, ResetPreview,
};
use crate::remote::{
    delete_remote_branch, fast_forward_branch, fetch, pull, push, push_preview, push_target,
    update_project, FetchOutcome, FetchRequest, PullRequest, PushOutcome, PushPreview, PushRequest,
    PushTarget, RemoteBranchDeleted, RemoteProgress, UpdateRequest,
};
use crate::repo::{open_repository, OperationKind, Repository};
use crate::stash::{
    list_stashes, pending_auto_stashes, restore_auto_stash, restore_pending_stash, AutoStash,
    LocalChanges, PendingStash, StashEntry,
};
use crate::status::{status, Status};

/// Délai de calme après le dernier changement sur disque avant de relire l'état.
pub const STATUS_DEBOUNCE: Duration = Duration::from_millis(300);

/// Ce que publie `julie-vcs` sur le bus, sous `Domain::Vcs`.
#[derive(Debug)]
pub enum VcsEvent {
    /// Dépôt trouvé ([`Vcs::open`]). Un `Status` suit.
    Opened(Repository),
    /// Le dossier n'est pas dans un dépôt Git (« Ce dossier n'est pas un dépôt Git. »).
    NoRepository {
        dir: PathBuf,
    },
    /// État relu (après [`Vcs::refresh_status`], un changement sur disque, ou
    /// toute opération qui écrit).
    Status(Status),
    Diff(DiffSides),
    Committed(CommitOutcome),
    Log(LogPage),
    CommitDetails(CommitDetails),
    Conflict(ConflictVersions),
    /// Fichier enregistré et marqué résolu ([`Vcs::resolve_conflict`]).
    Resolved {
        path: PathBuf,
    },
    /// Côté gardé ([`Vcs::accept_side`]).
    SideAccepted {
        side: ConflictSide,
        files: Vec<SideResolution>,
    },
    OperationContinued(ContinueOutcome),
    OperationAborted(OperationKind),
    /// Une commande Git commence (`result: None`) ou se termine : onglet Console.
    /// Publié hors tâche (`DomainEvent::task` vaut `None`).
    Command(CommandRecord),
    /// Une demande a échoué. La sortie de Git est dans l'erreur
    /// ([`VcsError::GitFailed`]). Jamais publié pour une annulation.
    Failed(Failure),

    // ---------------------------------------------------------- v1 : branches
    /// Branches relues ([`Vcs::load_branches`], et après toute opération qui
    /// touche aux branches ou au distant).
    Branches(BranchList),
    /// Vérification d'un nom à la frappe ; `problem: None` : valable.
    BranchNameChecked {
        name: String,
        problem: Option<BranchNameProblem>,
    },
    BranchCreated(CreateBranchOutcome),
    BranchRenamed(RenameBranchOutcome),
    /// Supprimée ; `tip` permet « Restaurer » ([`Vcs::restore_branch`]).
    BranchDeleted(DeleteBranchOutcome),
    BranchRestored {
        name: String,
        tip: String,
    },
    RemoteBranchDeleted(RemoteBranchDeleted),
    Switched(SwitchOutcome),
    /// Avertissement avant de quitter une HEAD détachée : commits sur aucune
    /// branche.
    DetachedCommits {
        count: u32,
    },
    // ---------------------------------------------------------- distant
    Fetched(FetchOutcome),
    /// Une branche non courante avancée sur son amont.
    BranchFastForwarded {
        branch: String,
        fetch: FetchOutcome,
    },
    /// « Mettre à jour le projet » terminé ou arrêté sur un conflit.
    Updated(OperationOutcome),
    Pulled(OperationOutcome),
    PushPreview(PushPreview),
    Pushed(PushOutcome),
    /// Progression d'une opération réseau (espacée de 100 ms au plus).
    Progress(RemoteProgress),
    // ---------------------------------------------------------- opérations
    /// Fusion, rebase, cherry-pick ou revert : terminé, ou arrêté sur un
    /// conflit (`finished: false`, `conflicts`).
    OperationRan(OperationOutcome),
    OperationSkipped(ContinueOutcome),
    ResetPreview(ResetPreview),
    ResetDone(ResetOutcome),
    Stashes(Vec<StashEntry>),
    /// Une mise de côté remise : sur demande ([`Vcs::restore_auto_stash`],
    /// [`Vcs::restore_pending_stash`]) **ou d'elle-même** juste après
    /// `OperationContinued` / `OperationSkipped` (opération terminée) et
    /// `OperationAborted`, quand l'opération avait mis de côté des changements.
    /// `restore` dit l'issue : `Restored` (entrée supprimée), `Conflicts` (liste
    /// des conflits), `Failed` ; `untracked_not_restored` à signaler.
    StashRestored(AutoStash),
    /// Les mises de côté de l'IDE encore dans `git stash list` (bandeau « Vos
    /// changements sont mis de côté »). Publié à l'ouverture, après **chaque**
    /// écriture, et par [`Vcs::load_pending_stashes`]. Liste vide : plus de
    /// bandeau.
    PendingStashes(Vec<PendingStash>),
    /// HEAD détachée (bandeau « Vos commits ne sont sur aucune branche » si
    /// `orphan_commits > 0`). Publié à l'ouverture et après chaque écriture
    /// **tant que HEAD est détachée** ; sur une branche, rien (se fier à
    /// `Status::head`).
    DetachedHead(DetachedHeadReport),
    /// Présence des commits dans la branche courante
    /// ([`Vcs::check_cherry_pick`]) : griser « Cherry-pick » si `is_present()`.
    CherryPickChecked(Vec<(String, CommitPresence)>),
    /// Amont posé ou retiré ([`Vcs::set_upstream`]).
    UpstreamChanged {
        branch: String,
        upstream: Option<String>,
    },
    // ---------------------------------------------------------- fichiers
    Staged {
        paths: Vec<PathBuf>,
    },
    Unstaged {
        paths: Vec<PathBuf>,
    },
    Discarded(DiscardOutcome),
    Moved {
        from: PathBuf,
        to: PathBuf,
    },
    IgnoreAdded(IgnoreAdded),
    /// La demande attend la fin de l'opération Git en cours (« En attente de
    /// l'opération Git en cours »). Elle partira d'elle-même.
    Waiting {
        action: Action,
    },
}

/// Quelle demande a échoué.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Open,
    Status,
    Diff,
    Commit,
    Log,
    CommitDetails,
    Conflict,
    Resolve,
    AcceptSide,
    Continue,
    Abort,
    // v1
    Branches,
    CheckBranchName,
    CreateBranch,
    RenameBranch,
    DeleteBranch,
    RestoreBranch,
    DeleteRemoteBranch,
    Switch,
    DetachedCommits,
    Fetch,
    FastForwardBranch,
    Update,
    Pull,
    PushPreview,
    Push,
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Skip,
    ResetPreview,
    Reset,
    Stashes,
    RestoreStash,
    PendingStashes,
    CheckCherryPick,
    SetUpstream,
    Stage,
    Unstage,
    Discard,
    Move,
    Ignore,
}

#[derive(Debug)]
pub struct Failure {
    pub action: Action,
    pub error: VcsError,
}

/// Le service Git, construit par `app`. Clonable (tous les clones partagent le
/// journal, le verrou d'écriture et l'anti-rebond).
#[derive(Clone)]
pub struct Vcs {
    tasks: Tasks,
    git: Git,
    settings: VcsSettings,
    /// Une seule opération d'écriture à la fois (pas de course sur `index.lock`).
    writes: Arc<Mutex<()>>,
    refresh: Arc<Mutex<RefreshState>>,
}

#[derive(Default)]
struct RefreshState {
    last_change: Option<Instant>,
    scheduled: bool,
}

impl Vcs {
    pub fn new(tasks: Tasks, settings: &VcsSettings) -> Self {
        Self::with_git(tasks, Git::new(settings)).with_settings(settings.clone())
    }

    /// Avec un [`Git`] déjà réglé (tests : configuration isolée). Les commandes
    /// seront publiées sur le bus de `tasks`. Réglages par défaut (branches
    /// protégées `main`, `master`).
    pub fn with_git(tasks: Tasks, git: Git) -> Self {
        let git = git.with_publisher(tasks.publisher().clone());
        Self {
            tasks,
            git,
            settings: VcsSettings::default(),
            writes: Arc::new(Mutex::new(())),
            refresh: Arc::new(Mutex::new(RefreshState::default())),
        }
    }

    /// Remplace les réglages (branches protégées…), après une modification
    /// de la page Git des réglages. `git_path` n'est relu qu'à la construction.
    pub fn with_settings(mut self, settings: VcsSettings) -> Self {
        self.settings = settings;
        self
    }

    pub fn settings(&self) -> &VcsSettings {
        &self.settings
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    /// Copie du journal des commandes (sans accès disque ni processus).
    pub fn command_log(&self) -> Vec<CommandRecord> {
        self.git.log().entries()
    }

    /// Cherche le dépôt de `dir` → `Opened` puis `Status`, ou `NoRepository`.
    pub fn open(&self, dir: PathBuf) -> TaskHandle {
        self.spawn(
            "vcs.open",
            Action::Open,
            false,
            move |git, ctx| match open_repository(git, &dir, ctx.cancel_token()) {
                Ok(repo) => {
                    ctx.publish(VcsEvent::Opened(repo.clone()));
                    ctx.publish(VcsEvent::Status(status(git, &repo, ctx.cancel_token())?));
                    publish_pending(git, &repo, ctx);
                    Ok(())
                }
                Err(VcsError::NotARepository(dir)) => {
                    ctx.publish(VcsEvent::NoRepository { dir });
                    Ok(())
                }
                Err(error) => Err(error),
            },
        )
    }

    /// Relit l'état → `Status`.
    pub fn refresh_status(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.status", Action::Status, false, move |git, ctx| {
            ctx.publish(VcsEvent::Status(status(git, &repo, ctx.cancel_token())?));
            Ok(())
        })
    }

    /// À appeler à chaque changement sur disque (`Event::FilesChangedOnDisk`) et
    /// au retour du focus (`paths` vide). Relit l'état [`STATUS_DEBOUNCE`] après le
    /// dernier changement pertinent, une seule tâche à la fois ; rend la tâche
    /// lancée, ou `None` si une relecture est déjà prévue ou si aucun chemin
    /// n'est pertinent.
    pub fn notify_disk_change(&self, repo: &Repository, paths: &[PathBuf]) -> Option<TaskHandle> {
        if !paths.is_empty() && !paths.iter().any(|path| is_relevant_change(repo, path)) {
            return None;
        }
        {
            let mut state = lock(&self.refresh);
            state.last_change = Some(Instant::now());
            if state.scheduled {
                return None;
            }
            state.scheduled = true;
        }
        let repo = repo.clone();
        let refresh = Arc::clone(&self.refresh);
        let reset = Arc::clone(&self.refresh);
        let handle = self.spawn("vcs.status", Action::Status, false, move |git, ctx| {
            let result = debounced_refresh(git, &repo, &refresh, ctx);
            if result.is_err() {
                lock(&reset).scheduled = false;
            }
            result
        });
        Some(handle)
    }

    /// Charge les deux volets d'un diff → `Diff`.
    pub fn load_diff(&self, repo: &Repository, request: DiffRequest) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.diff", Action::Diff, false, move |git, ctx| {
            ctx.publish(VcsEvent::Diff(load_diff(
                git,
                &repo,
                request,
                ctx.cancel_token(),
            )?));
            Ok(())
        })
    }

    /// Valide → `Committed` puis `Status`. En cas d'échec → `Failed` (rien n'est
    /// validé ; le message reste à l'interface).
    pub fn commit(&self, repo: &Repository, request: CommitRequest) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.commit", Action::Commit, true, move |git, ctx| {
            let result = commit(git, &repo, &request, ctx.cancel_token());
            then_status(git, &repo, ctx, result, VcsEvent::Committed)
        })
    }

    /// Une page du journal → `Log`.
    pub fn load_log(&self, repo: &Repository, query: LogQuery) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.log", Action::Log, false, move |git, ctx| {
            ctx.publish(VcsEvent::Log(log(git, &repo, &query, ctx.cancel_token())?));
            Ok(())
        })
    }

    /// Détail d'un commit → `CommitDetails`.
    pub fn load_commit(&self, repo: &Repository, commit: String) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.commit_details",
            Action::CommitDetails,
            false,
            move |git, ctx| {
                let details = commit_details(git, &repo, &commit, ctx.cancel_token())?;
                ctx.publish(VcsEvent::CommitDetails(details));
                Ok(())
            },
        )
    }

    /// Base / nôtre / leur d'un fichier en conflit → `Conflict`.
    pub fn load_conflict(&self, repo: &Repository, path: PathBuf) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.conflict", Action::Conflict, false, move |git, ctx| {
            let versions = conflict_versions(git, &repo, &path, ctx.cancel_token())?;
            ctx.publish(VcsEvent::Conflict(versions));
            Ok(())
        })
    }

    /// Écrit le résultat d'une fusion et marque le fichier résolu → `Resolved`
    /// puis `Status`. `expected_work_tree` : les octets lus à l'ouverture
    /// (`ConflictVersions::work_tree`) ; s'ils ont changé, rien n'est écrit.
    pub fn resolve_conflict(
        &self,
        repo: &Repository,
        path: PathBuf,
        content: Vec<u8>,
        expected_work_tree: Option<Vec<u8>>,
    ) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.resolve", Action::Resolve, true, move |git, ctx| {
            let result = resolve_with_content(
                git,
                &repo,
                &path,
                &content,
                expected_work_tree.as_deref(),
                ctx.cancel_token(),
            );
            then_status(git, &repo, ctx, result, |()| VcsEvent::Resolved { path })
        })
    }

    /// Garde un côté entier (peut supprimer des fichiers : confirmer avant) →
    /// `SideAccepted` puis `Status`.
    pub fn accept_side(
        &self,
        repo: &Repository,
        paths: Vec<PathBuf>,
        side: ConflictSide,
    ) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.accept_side",
            Action::AcceptSide,
            true,
            move |git, ctx| {
                let result = accept_side(git, &repo, &paths, side, ctx.cancel_token());
                then_status(git, &repo, ctx, result, |files| VcsEvent::SideAccepted {
                    side,
                    files,
                })
            },
        )
    }

    /// Conclut la fusion (ou poursuit rebase / cherry-pick / revert) →
    /// `OperationContinued` puis `Status`.
    pub fn continue_operation(&self, repo: &Repository, message: Option<String>) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.continue", Action::Continue, true, move |git, ctx| {
            let result = continue_operation(git, &repo, message.as_deref(), ctx.cancel_token());
            if let Ok(outcome) = &result {
                ctx.publish(VcsEvent::OperationContinued(outcome.clone()));
                if let Some(stash) = &outcome.stash {
                    ctx.publish(VcsEvent::StashRestored(stash.clone()));
                }
            }
            let outcome = result.map(|_| ());
            publish_status(git, &repo, ctx);
            publish_pending(git, &repo, ctx);
            outcome
        })
    }

    /// Abandonne l'opération en cours (**défait la fusion et ses résolutions**) →
    /// `OperationAborted`, puis `StashRestored` si l'opération avait mis de
    /// côté des changements (ils sont remis), puis `Status`.
    pub fn abort_operation(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.abort", Action::Abort, true, move |git, ctx| {
            let result = abort_operation_restoring(git, &repo, ctx.cancel_token());
            if let Ok(outcome) = &result {
                ctx.publish(VcsEvent::OperationAborted(outcome.kind));
                if let Some(stash) = &outcome.stash {
                    ctx.publish(VcsEvent::StashRestored(stash.clone()));
                }
            }
            let outcome = result.map(|_| ());
            publish_status(git, &repo, ctx);
            publish_pending(git, &repo, ctx);
            outcome
        })
    }

    // ------------------------------------------------------------------ branches

    /// Branches locales et distantes, distants, récentes → `Branches`.
    pub fn load_branches(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.branches", Action::Branches, false, move |git, ctx| {
            ctx.publish(VcsEvent::Branches(list_branches(
                git,
                &repo,
                ctx.cancel_token(),
            )?));
            Ok(())
        })
    }

    /// Vérifie un nom à la frappe → `BranchNameChecked` (annuler la
    /// vérification précédente). `ignore` : la branche qu'on renomme.
    pub fn check_branch_name(
        &self,
        repo: &Repository,
        name: String,
        ignore: Option<String>,
    ) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.branch_name",
            Action::CheckBranchName,
            false,
            move |git, ctx| {
                let problem =
                    check_branch_name(git, &repo, &name, ignore.as_deref(), ctx.cancel_token())?;
                ctx.publish(VcsEvent::BranchNameChecked { name, problem });
                Ok(())
            },
        )
    }

    /// → `BranchCreated` puis `Status` et `Branches`.
    pub fn create_branch(&self, repo: &Repository, request: CreateBranchRequest) -> TaskHandle {
        self.write(
            "vcs.branch_create",
            Action::CreateBranch,
            repo,
            move |git, repo, cancel| {
                create_branch(git, repo, &request, cancel).map(VcsEvent::BranchCreated)
            },
        )
    }

    /// → `BranchRenamed` puis `Status` et `Branches`.
    pub fn rename_branch(&self, repo: &Repository, old: String, new: String) -> TaskHandle {
        self.write(
            "vcs.branch_rename",
            Action::RenameBranch,
            repo,
            move |git, repo, cancel| {
                rename_branch(git, repo, &old, &new, cancel).map(VcsEvent::BranchRenamed)
            },
        )
    }

    /// `git branch -d` → `BranchDeleted`, ou `Failed` avec
    /// [`VcsError::BranchNotMerged`] (seconde confirmation, puis
    /// [`Vcs::force_delete_branch`]).
    pub fn delete_branch(&self, repo: &Repository, name: String) -> TaskHandle {
        self.write(
            "vcs.branch_delete",
            Action::DeleteBranch,
            repo,
            move |git, repo, cancel| {
                delete_branch(git, repo, &name, cancel).map(VcsEvent::BranchDeleted)
            },
        )
    }

    /// **Destructif** : `git branch -D` → `BranchDeleted`.
    pub fn force_delete_branch(&self, repo: &Repository, name: String) -> TaskHandle {
        self.write(
            "vcs.branch_delete",
            Action::DeleteBranch,
            repo,
            move |git, repo, cancel| {
                force_delete_branch(git, repo, &name, cancel).map(VcsEvent::BranchDeleted)
            },
        )
    }

    /// « Restaurer » après suppression → `BranchRestored`.
    pub fn restore_branch(&self, repo: &Repository, name: String, tip: String) -> TaskHandle {
        self.write(
            "vcs.branch_restore",
            Action::RestoreBranch,
            repo,
            move |git, repo, cancel| {
                restore_branch(git, repo, &name, &tip, cancel)
                    .map(|()| VcsEvent::BranchRestored { name, tip })
            },
        )
    }

    /// « Restaurer » avec l'amont d'avant la suppression (le `BranchDeleted`
    /// reçu) → `BranchRestored`. À préférer à [`Vcs::restore_branch`].
    pub fn restore_deleted_branch(
        &self,
        repo: &Repository,
        deleted: DeleteBranchOutcome,
    ) -> TaskHandle {
        self.write(
            "vcs.branch_restore",
            Action::RestoreBranch,
            repo,
            move |git, repo, cancel| {
                restore_deleted_branch(git, repo, &deleted, cancel).map(|()| {
                    VcsEvent::BranchRestored {
                        name: deleted.name,
                        tip: deleted.tip,
                    }
                })
            },
        )
    }

    /// « Choisir l'amont… » (`Some("origin/x")`) ou « Retirer l'amont »
    /// (`None`) → `UpstreamChanged { branch, upstream }` puis `Status` et
    /// `Branches`. Amont inexistant : `Failed(BranchNotFound)`.
    pub fn set_upstream(
        &self,
        repo: &Repository,
        branch: String,
        upstream: Option<String>,
    ) -> TaskHandle {
        self.write(
            "vcs.upstream",
            Action::SetUpstream,
            repo,
            move |git, repo, cancel| {
                match &upstream {
                    Some(upstream) => set_upstream(git, repo, &branch, upstream, cancel)?,
                    None => unset_upstream(git, repo, &branch, cancel)?,
                }
                let upstream = upstream_short(git, repo, &branch, cancel)?;
                Ok(VcsEvent::UpstreamChanged { branch, upstream })
            },
        )
    }

    /// **Destructif pour tout le monde** : supprime la branche sur le distant
    /// (refusé si protégée) → `RemoteBranchDeleted`.
    pub fn delete_remote_branch(
        &self,
        repo: &Repository,
        remote: String,
        branch: String,
    ) -> TaskHandle {
        let protected = self.settings.protected_branches.clone();
        self.network(
            "vcs.remote_branch_delete",
            Action::DeleteRemoteBranch,
            repo,
            move |git, repo, progress, cancel| {
                delete_remote_branch(git, repo, &remote, &branch, &protected, progress, cancel)
                    .map(VcsEvent::RemoteBranchDeleted)
            },
        )
    }

    /// Bascule → `Switched` (ou `Failed` avec [`VcsError::WouldOverwrite`] :
    /// proposer « Mettre de côté et basculer » ou « Forcer la bascule » ; avec
    /// [`VcsError::DetachedCommitsLeft`] : avertir, « Créer une branche
    /// d'abord » ou « Basculer quand même » = [`Vcs::switch_branch_with`]).
    pub fn switch_branch(&self, repo: &Repository, request: SwitchRequest) -> TaskHandle {
        self.switch_branch_with(repo, request, SwitchOptions::default())
    }

    /// Bascule avec des options accordées par l'utilisatrice → `Switched`.
    pub fn switch_branch_with(
        &self,
        repo: &Repository,
        request: SwitchRequest,
        options: SwitchOptions,
    ) -> TaskHandle {
        self.write(
            "vcs.switch",
            Action::Switch,
            repo,
            move |git, repo, cancel| {
                switch_branch_with(git, repo, &request, options, cancel).map(VcsEvent::Switched)
            },
        )
    }

    /// **Destructif** : « Forcer la bascule » en jetant les changements des
    /// seuls fichiers de `discard` → `Switched` (`discarded` : ce qui a
    /// réellement été jeté).
    pub fn switch_discarding_changes(
        &self,
        repo: &Repository,
        target: SwitchTarget,
        discard: OverwriteConflict,
    ) -> TaskHandle {
        self.switch_discarding_changes_with(repo, target, discard, SwitchOptions::default())
    }

    /// **Destructif** : [`Vcs::switch_discarding_changes`] avec options.
    pub fn switch_discarding_changes_with(
        &self,
        repo: &Repository,
        target: SwitchTarget,
        discard: OverwriteConflict,
        options: SwitchOptions,
    ) -> TaskHandle {
        self.write(
            "vcs.switch",
            Action::Switch,
            repo,
            move |git, repo, cancel| {
                switch_discarding_changes_with(git, repo, &target, &discard, options, cancel)
                    .map(VcsEvent::Switched)
            },
        )
    }

    /// Commits d'une HEAD détachée qui ne sont sur aucune branche →
    /// `DetachedCommits`.
    pub fn check_detached_commits(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.detached_commits",
            Action::DetachedCommits,
            false,
            move |git, ctx| {
                let count = commits_only_on_detached_head(git, &repo, ctx.cancel_token())?;
                ctx.publish(VcsEvent::DetachedCommits { count });
                Ok(())
            },
        )
    }

    // ------------------------------------------------------------------ distant

    /// Récupérer → `Progress`…, `Fetched`, puis `Status` et `Branches`.
    pub fn fetch(&self, repo: &Repository, request: FetchRequest) -> TaskHandle {
        self.network(
            "vcs.fetch",
            Action::Fetch,
            repo,
            move |git, repo, progress, cancel| {
                fetch(git, repo, &request, progress, cancel).map(VcsEvent::Fetched)
            },
        )
    }

    /// Avance une branche non courante sur son amont → `BranchFastForwarded`.
    pub fn fast_forward_branch(&self, repo: &Repository, branch: String) -> TaskHandle {
        self.network(
            "vcs.branch_update",
            Action::FastForwardBranch,
            repo,
            move |git, repo, progress, cancel| {
                fast_forward_branch(git, repo, &branch, progress, cancel)
                    .map(|fetch| VcsEvent::BranchFastForwarded { branch, fetch })
            },
        )
    }

    /// « Mettre à jour le projet » → `Updated` (ou `Failed` avec
    /// [`VcsError::NoUpstream`], [`VcsError::UpstreamGone`]…).
    pub fn update_project(&self, repo: &Repository, request: UpdateRequest) -> TaskHandle {
        self.network(
            "vcs.update",
            Action::Update,
            repo,
            move |git, repo, progress, cancel| {
                update_project(git, repo, &request, progress, cancel).map(VcsEvent::Updated)
            },
        )
    }

    /// → `Pulled`.
    pub fn pull(&self, repo: &Repository, request: PullRequest) -> TaskHandle {
        self.network(
            "vcs.pull",
            Action::Pull,
            repo,
            move |git, repo, progress, cancel| {
                pull(git, repo, &request, progress, cancel).map(VcsEvent::Pulled)
            },
        )
    }

    /// Dialogue de push : cible par défaut de `branch` (la courante si
    /// `None`) ou `target` donnée, commits sortants → `PushPreview`. Sans
    /// réseau.
    pub fn load_push_preview(
        &self,
        repo: &Repository,
        branch: Option<String>,
        target: Option<PushTarget>,
    ) -> TaskHandle {
        let repo = repo.clone();
        let protected = self.settings.protected_branches.clone();
        self.spawn(
            "vcs.push_preview",
            Action::PushPreview,
            false,
            move |git, ctx| {
                let target = match target {
                    Some(target) => target,
                    None => push_target(git, &repo, branch.as_deref(), ctx.cancel_token())?,
                };
                let preview = push_preview(git, &repo, &target, &protected, ctx.cancel_token())?;
                ctx.publish(VcsEvent::PushPreview(preview));
                Ok(())
            },
        )
    }

    /// Pousser → `Progress`…, `Pushed` (ou `Failed` avec
    /// [`VcsError::PushRejected`], [`VcsError::AuthenticationFailed`]…).
    pub fn push(&self, repo: &Repository, request: PushRequest) -> TaskHandle {
        let protected = self.settings.protected_branches.clone();
        self.network(
            "vcs.push",
            Action::Push,
            repo,
            move |git, repo, progress, cancel| {
                push(git, repo, &request, &protected, progress, cancel).map(VcsEvent::Pushed)
            },
        )
    }

    // ------------------------------------------------------------------ opérations

    /// → `OperationRan`.
    pub fn merge(&self, repo: &Repository, request: MergeRequest) -> TaskHandle {
        self.write(
            "vcs.merge",
            Action::Merge,
            repo,
            move |git, repo, cancel| merge(git, repo, &request, cancel).map(VcsEvent::OperationRan),
        )
    }

    /// → `OperationRan`.
    pub fn rebase(&self, repo: &Repository, request: RebaseRequest) -> TaskHandle {
        self.write(
            "vcs.rebase",
            Action::Rebase,
            repo,
            move |git, repo, cancel| {
                rebase(git, repo, &request, cancel).map(VcsEvent::OperationRan)
            },
        )
    }

    /// Commits du plus ancien au plus récent → `OperationRan`.
    pub fn cherry_pick(
        &self,
        repo: &Repository,
        commits: Vec<String>,
        local_changes: LocalChanges,
    ) -> TaskHandle {
        self.write(
            "vcs.cherry_pick",
            Action::CherryPick,
            repo,
            move |git, repo, cancel| {
                cherry_pick(git, repo, &commits, local_changes, cancel).map(VcsEvent::OperationRan)
            },
        )
    }

    /// Les commits sont-ils déjà dans la branche courante ? →
    /// `CherryPickChecked` (griser « Cherry-pick » : « déjà sur la branche
    /// courante »).
    pub fn check_cherry_pick(&self, repo: &Repository, commits: Vec<String>) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.cherry_pick_check",
            Action::CheckCherryPick,
            false,
            move |git, ctx| {
                let presence = commits_presence(git, &repo, &commits, ctx.cancel_token())?;
                ctx.publish(VcsEvent::CherryPickChecked(presence));
                Ok(())
            },
        )
    }

    /// → `OperationRan`.
    pub fn revert_commits(
        &self,
        repo: &Repository,
        commits: Vec<String>,
        local_changes: LocalChanges,
    ) -> TaskHandle {
        self.write(
            "vcs.revert",
            Action::Revert,
            repo,
            move |git, repo, cancel| {
                revert_commits(git, repo, &commits, local_changes, cancel)
                    .map(VcsEvent::OperationRan)
            },
        )
    }

    /// Passe le commit en cours (abandonne ses changements) →
    /// `OperationSkipped`, puis `StashRestored` si l'opération s'est terminée
    /// et avait mis de côté des changements.
    pub fn skip_operation(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.skip", Action::Skip, true, move |git, ctx| {
            let result = skip_operation(git, &repo, ctx.cancel_token());
            if let Ok(outcome) = &result {
                ctx.publish(VcsEvent::OperationSkipped(outcome.clone()));
                if let Some(stash) = &outcome.stash {
                    ctx.publish(VcsEvent::StashRestored(stash.clone()));
                }
            }
            let outcome = result.map(|_| ());
            publish_status(git, &repo, ctx);
            publish_branches(git, &repo, ctx);
            publish_pending(git, &repo, ctx);
            outcome
        })
    }

    /// → `ResetPreview`.
    pub fn load_reset_preview(&self, repo: &Repository, target: String) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.reset_preview",
            Action::ResetPreview,
            false,
            move |git, ctx| {
                ctx.publish(VcsEvent::ResetPreview(reset_preview(
                    git,
                    &repo,
                    &target,
                    ctx.cancel_token(),
                )?));
                Ok(())
            },
        )
    }

    /// `reset --soft|--mixed` → `ResetDone`.
    pub fn reset(&self, repo: &Repository, target: String, mode: ResetMode) -> TaskHandle {
        self.write(
            "vcs.reset",
            Action::Reset,
            repo,
            move |git, repo, cancel| {
                reset(git, repo, &target, mode, cancel).map(VcsEvent::ResetDone)
            },
        )
    }

    /// **Destructif** : `reset --hard` → `ResetDone`.
    pub fn reset_hard(&self, repo: &Repository, target: String, stash_first: bool) -> TaskHandle {
        self.write(
            "vcs.reset",
            Action::Reset,
            repo,
            move |git, repo, cancel| {
                reset_hard(git, repo, &target, stash_first, cancel).map(VcsEvent::ResetDone)
            },
        )
    }

    /// `git stash list` → `Stashes`.
    pub fn load_stashes(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.stashes", Action::Stashes, false, move |git, ctx| {
            ctx.publish(VcsEvent::Stashes(list_stashes(
                git,
                &repo,
                ctx.cancel_token(),
            )?));
            Ok(())
        })
    }

    /// Restaure une mise de côté gardée → `StashRestored`.
    pub fn restore_auto_stash(&self, repo: &Repository, commit: String) -> TaskHandle {
        self.write(
            "vcs.stash_restore",
            Action::RestoreStash,
            repo,
            move |git, repo, cancel| {
                restore_auto_stash(git, repo, &commit, cancel).map(VcsEvent::StashRestored)
            },
        )
    }

    /// Les mises de côté de l'IDE en attente → `PendingStashes`.
    pub fn load_pending_stashes(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn(
            "vcs.pending_stashes",
            Action::PendingStashes,
            false,
            move |git, ctx| {
                let pending = pending_auto_stashes(git, &repo, ctx.cancel_token())?;
                ctx.publish(VcsEvent::PendingStashes(pending));
                Ok(())
            },
        )
    }

    /// Remet la mise de côté liée à une opération terminée ailleurs (état
    /// `OperationOver`) → `StashRestored`, ou rien s'il n'y en a pas ;
    /// `Failed(OperationInProgress)` si l'opération n'est pas finie.
    pub fn restore_pending_stash(&self, repo: &Repository) -> TaskHandle {
        self.write(
            "vcs.stash_restore",
            Action::RestoreStash,
            repo,
            move |git, repo, cancel| {
                // Rien à remettre : on republie la liste (sans changement).
                match restore_pending_stash(git, repo, cancel)? {
                    Some(stash) => Ok(VcsEvent::StashRestored(stash)),
                    None => Ok(VcsEvent::PendingStashes(pending_auto_stashes(
                        git, repo, cancel,
                    )?)),
                }
            },
        )
    }

    // ------------------------------------------------------------------ fichiers

    /// « Ajouter à Git » → `Staged`.
    pub fn stage(&self, repo: &Repository, paths: Vec<PathBuf>) -> TaskHandle {
        self.write(
            "vcs.stage",
            Action::Stage,
            repo,
            move |git, repo, cancel| {
                stage(git, repo, &paths, cancel).map(|()| VcsEvent::Staged { paths })
            },
        )
    }

    /// « Retirer de l'index » → `Unstaged`.
    pub fn unstage(&self, repo: &Repository, paths: Vec<PathBuf>) -> TaskHandle {
        self.write(
            "vcs.unstage",
            Action::Unstage,
            repo,
            move |git, repo, cancel| {
                unstage(git, repo, &paths, cancel).map(|()| VcsEvent::Unstaged { paths })
            },
        )
    }

    /// **Destructif** : « Annuler les modifications » → `Discarded`.
    pub fn discard_changes(&self, repo: &Repository, paths: Vec<PathBuf>) -> TaskHandle {
        self.write(
            "vcs.discard",
            Action::Discard,
            repo,
            move |git, repo, cancel| {
                discard_changes(git, repo, &paths, cancel).map(VcsEvent::Discarded)
            },
        )
    }

    /// `git mv` → `Moved`.
    pub fn move_path(&self, repo: &Repository, from: PathBuf, to: PathBuf) -> TaskHandle {
        self.write("vcs.move", Action::Move, repo, move |git, repo, cancel| {
            move_path(git, repo, &from, &to, cancel).map(|()| VcsEvent::Moved { from, to })
        })
    }

    /// Ajout au `.gitignore` → `IgnoreAdded`.
    pub fn add_to_gitignore(&self, repo: &Repository, rule: IgnoreRule) -> TaskHandle {
        self.write(
            "vcs.ignore",
            Action::Ignore,
            repo,
            move |git, repo, cancel| {
                add_to_gitignore(git, repo, &rule, cancel).map(VcsEvent::IgnoreAdded)
            },
        )
    }

    /// Une écriture : sérialisée, puis `Status` et `Branches` relus, réussite
    /// ou non.
    fn write<F>(&self, kind: &'static str, action: Action, repo: &Repository, work: F) -> TaskHandle
    where
        F: FnOnce(&Git, &Repository, &CancelToken) -> Result<VcsEvent> + Send + 'static,
    {
        let repo = repo.clone();
        self.spawn(kind, action, true, move |git, ctx| {
            let result = work(git, &repo, ctx.cancel_token());
            let outcome = result.map(|event| ctx.publish(event));
            publish_status(git, &repo, ctx);
            publish_branches(git, &repo, ctx);
            publish_pending(git, &repo, ctx);
            outcome
        })
    }

    /// Une opération réseau : comme [`Vcs::write`], avec la progression
    /// publiée (`Progress`, espacée) et passée à la tâche.
    fn network<F>(
        &self,
        kind: &'static str,
        action: Action,
        repo: &Repository,
        work: F,
    ) -> TaskHandle
    where
        F: FnOnce(
                &Git,
                &Repository,
                &mut dyn FnMut(RemoteProgress),
                &CancelToken,
            ) -> Result<VcsEvent>
            + Send
            + 'static,
    {
        let repo = repo.clone();
        self.spawn(kind, action, true, move |git, ctx| {
            let mut last: Option<Instant> = None;
            let mut on_progress = |progress: RemoteProgress| {
                ctx.progress(progress.done, progress.total);
                if last.is_some_and(|at| at.elapsed() < PROGRESS_SPACING) {
                    return;
                }
                last = Some(Instant::now());
                ctx.publish(VcsEvent::Progress(progress));
            };
            let result = work(git, &repo, &mut on_progress, ctx.cancel_token());
            let outcome = result.map(|event| ctx.publish(event));
            publish_status(git, &repo, ctx);
            publish_branches(git, &repo, ctx);
            publish_pending(git, &repo, ctx);
            outcome
        })
    }

    fn spawn<F>(&self, kind: &'static str, action: Action, exclusive: bool, work: F) -> TaskHandle
    where
        F: FnOnce(&Git, &TaskContext) -> Result<()> + Send + 'static,
    {
        let git = self.git.clone();
        let writes = Arc::clone(&self.writes);
        self.tasks.spawn(Domain::Vcs, kind, move |ctx| {
            let _guard = exclusive.then(|| match writes.try_lock() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    ctx.publish(VcsEvent::Waiting { action });
                    lock(&writes)
                }
            });
            match work(&git, ctx) {
                Ok(()) => Ok(()),
                Err(error) if error.is_cancelled() => Err(error.into()),
                Err(error) => {
                    let message = error.to_string();
                    ctx.publish(VcsEvent::Failed(Failure { action, error }));
                    Err(message.into())
                }
            }
        })
    }
}

/// Après une écriture, l'état est relu quoi qu'il arrive (même après un échec :
/// un `git add` a pu passer avant). Un échec de relecture n'efface pas celui de
/// l'opération : on le tait.
fn publish_status(git: &Git, repo: &Repository, ctx: &TaskContext) {
    if let Ok(status) = status(git, repo, &CancelToken::new()) {
        ctx.publish(VcsEvent::Status(status));
    }
}

/// Mises de côté en attente, et HEAD détachée s'il y a lieu, après une
/// écriture ; un échec de relecture est tu.
fn publish_pending(git: &Git, repo: &Repository, ctx: &TaskContext) {
    if let Ok(pending) = pending_auto_stashes(git, repo, &CancelToken::new()) {
        ctx.publish(VcsEvent::PendingStashes(pending));
    }
    publish_detached(git, repo, ctx);
}

/// `DetachedHead` si HEAD est détachée (aucune commande sinon).
fn publish_detached(git: &Git, repo: &Repository, ctx: &TaskContext) {
    if let Ok(Some(report)) = detached_head_report(git, repo, &CancelToken::new()) {
        ctx.publish(VcsEvent::DetachedHead(report));
    }
}

/// Les branches relues après une écriture ; un échec de relecture est tu.
fn publish_branches(git: &Git, repo: &Repository, ctx: &TaskContext) {
    if let Ok(list) = list_branches(git, repo, &CancelToken::new()) {
        ctx.publish(VcsEvent::Branches(list));
    }
}

/// Écart minimal entre deux `VcsEvent::Progress`.
const PROGRESS_SPACING: Duration = Duration::from_millis(100);

/// Publie le résultat d'une écriture, puis l'état relu.
fn then_status<T>(
    git: &Git,
    repo: &Repository,
    ctx: &TaskContext,
    result: Result<T>,
    event: impl FnOnce(T) -> VcsEvent,
) -> Result<()> {
    let outcome = result.map(|value| ctx.publish(event(value)));
    publish_status(git, repo, ctx);
    // Commit, résolution, côté gardé : rien ne touche aux mises de côté, mais
    // un commit sur une HEAD détachée fait naître le bandeau.
    publish_detached(git, repo, ctx);
    outcome
}

fn debounced_refresh(
    git: &Git,
    repo: &Repository,
    refresh: &Mutex<RefreshState>,
    ctx: &TaskContext,
) -> Result<()> {
    loop {
        // Attendre le calme.
        loop {
            ctx.check_cancelled()?;
            let last = lock(refresh).last_change;
            match last {
                Some(at) if at.elapsed() < STATUS_DEBOUNCE => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => break,
            }
        }
        lock(refresh).last_change = None;
        ctx.publish(VcsEvent::Status(status(git, repo, ctx.cancel_token())?));
        // Un changement arrivé pendant la lecture : on recommence.
        let mut state = lock(refresh);
        if state.last_change.is_none() {
            state.scheduled = false;
            return Ok(());
        }
    }
}

/// Un changement sur ce chemin peut-il changer l'état Git ? Écarte les objets,
/// journaux de références et verrous de `.git`, et le dossier `.juliestorm`.
pub fn is_relevant_change(repo: &Repository, path: &Path) -> bool {
    let Ok(repo_path) = repo.to_repo_path(path) else {
        // Hors de l'arbre de travail : peut-être le dossier Git d'un arbre lié.
        return path.starts_with(repo.git_dir()) && !is_git_noise(path);
    };
    let first = repo_path.split('/').next().unwrap_or_default();
    if first == julie_core::project::DATA_DIR_NAME {
        return false;
    }
    if first == ".git" {
        return !is_git_noise(path);
    }
    true
}

fn is_git_noise(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    text.contains("/.git/objects/")
        || text.contains("/.git/logs/")
        || text.ends_with(".lock")
        || text.contains("/.git/fsmonitor")
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
