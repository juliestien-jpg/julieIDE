//! Le service asynchrone : chaque méthode lance une tâche de fond (`Domain::Vcs`)
//! et rend aussitôt la main ; les résultats arrivent sur le bus en [`VcsEvent`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use julie_core::{CancelToken, Domain, TaskContext, TaskHandle, Tasks};

use crate::commit::{commit, CommitOutcome, CommitRequest};
use crate::conflict::{
    abort_operation, accept_side, conflict_versions, continue_operation, resolve_with_content,
    ConflictSide, ConflictVersions, ContinueOutcome, SideResolution,
};
use crate::content::{load_diff, DiffRequest, DiffSides};
use crate::error::{Result, VcsError};
use crate::git::{CommandRecord, Git, VcsSettings};
use crate::history::{commit_details, log, CommitDetails, LogPage, LogQuery};
use crate::repo::{open_repository, OperationKind, Repository};
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
        Self::with_git(tasks, Git::new(settings))
    }

    /// Avec un [`Git`] déjà réglé (tests : configuration isolée). Les commandes
    /// seront publiées sur le bus de `tasks`.
    pub fn with_git(tasks: Tasks, git: Git) -> Self {
        let git = git.with_publisher(tasks.publisher().clone());
        Self {
            tasks,
            git,
            writes: Arc::new(Mutex::new(())),
            refresh: Arc::new(Mutex::new(RefreshState::default())),
        }
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
            then_status(git, &repo, ctx, result, VcsEvent::OperationContinued)
        })
    }

    /// Abandonne l'opération en cours (**défait la fusion et ses résolutions**) →
    /// `OperationAborted` puis `Status`.
    pub fn abort_operation(&self, repo: &Repository) -> TaskHandle {
        let repo = repo.clone();
        self.spawn("vcs.abort", Action::Abort, true, move |git, ctx| {
            let result = abort_operation(git, &repo, ctx.cancel_token());
            then_status(git, &repo, ctx, result, VcsEvent::OperationAborted)
        })
    }

    fn spawn<F>(&self, kind: &'static str, action: Action, exclusive: bool, work: F) -> TaskHandle
    where
        F: FnOnce(&Git, &TaskContext) -> Result<()> + Send + 'static,
    {
        let git = self.git.clone();
        let writes = Arc::clone(&self.writes);
        self.tasks.spawn(Domain::Vcs, kind, move |ctx| {
            let _guard = exclusive.then(|| lock(&writes));
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
