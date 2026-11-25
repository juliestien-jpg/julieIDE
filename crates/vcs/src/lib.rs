//! Git piloté par la ligne de commande : état, diff, commit, journal et graphe,
//! branches, distant, fusion / rebase / cherry-pick / revert / reset, conflits,
//! sous-menu Git des fichiers.
//! Propriétaire : dev-git. Ne dépend que de `julie-core` (ni egui, ni autre crate
//! métier). Contrat résumé dans `docs/contrats.md` (section `julie-vcs`).
//!
//! # Deux façons de s'en servir
//!
//! **Depuis l'interface : le service [`Vcs`]**. Chaque méthode lance une tâche de
//! fond (`Domain::Vcs`) et rend aussitôt un [`julie_core::TaskHandle`] ; le résultat
//! arrive sur le bus en [`VcsEvent`]. Aucune commande Git ne tourne sur le fil de
//! l'interface.
//!
//! ```ignore
//! let settings: VcsSettings = settings.section("vcs")?;
//! let vcs = Vcs::new(tasks.clone(), &settings);
//! vcs.open(project.root().to_path_buf());          // -> Opened(repo) + Status, ou NoRepository
//! // une fois `repo` reçu :
//! vcs.refresh_status(&repo);                        // -> Status
//! vcs.notify_disk_change(&repo, &paths);            // anti-rebond 300 ms -> Status
//! vcs.load_diff(&repo, DiffRequest::local_change(&file));          // -> Diff
//! vcs.commit(&repo, CommitRequest { paths, message, amend: false }); // -> Committed + Status
//! vcs.load_log(&repo, LogQuery::default());         // -> Log (500 par page)
//! vcs.load_commit(&repo, id);                       // -> CommitDetails
//! vcs.load_conflict(&repo, path);                   // -> Conflict (base / nôtre / leur)
//! vcs.resolve_conflict(&repo, path, bytes, Some(work_tree_bytes)); // -> Resolved + Status
//! vcs.accept_side(&repo, paths, ConflictSide::Theirs);             // -> SideAccepted + Status
//! vcs.continue_operation(&repo, None);              // -> OperationContinued + Status
//! vcs.abort_operation(&repo);                       // -> OperationAborted + Status
//! ```
//!
//! Côté `app`, une fois par image :
//!
//! ```ignore
//! Event::Domain(event) if event.domain == Domain::Vcs => match event.get::<VcsEvent>() {
//!     Some(VcsEvent::Status(status)) => changes_view.show(status),
//!     Some(VcsEvent::Command(record)) => console.upsert(record),   // même `id` au début et à la fin
//!     Some(VcsEvent::Failed(failure)) => show_git_error(failure),  // sortie de Git dans l'erreur
//!     // …
//! }
//! ```
//!
//! Une erreur publie `VcsEvent::Failed { action, error }` **et** termine la tâche
//! en `Failed` ; une annulation ne publie rien de plus que `Cancelled`. Toute
//! opération qui écrit publie ensuite un `Status` frais, réussite ou non. Les
//! écritures sont sérialisées entre elles ; une écriture qui doit attendre
//! publie d'abord `Waiting { action }` (« En attente de l'opération Git en
//! cours »).
//!
//! # Branches, distant, opérations (v1)
//!
//! Les écritures de la v1 publient leur résultat, puis `Status` **et**
//! `Branches` relus (réussite ou non). Les opérations réseau publient en plus
//! `Progress(RemoteProgress)` (espacé de 100 ms) et la progression de la tâche.
//!
//! ```ignore
//! // Widget de branche et fenêtre surgissante
//! vcs.load_branches(&repo);                                   // -> Branches(BranchList)
//! branch_tree(&names);                                        // pur : dossiers par préfixe
//! vcs.check_branch_name(&repo, name, None);                   // -> BranchNameChecked { problem }
//! vcs.create_branch(&repo, CreateBranchRequest { name, start, switch: true, local_changes });
//! vcs.switch_branch(&repo, SwitchRequest { target: SwitchTarget::Remote("origin/x".into()),
//!                                          local_changes: LocalChanges::Keep });
//!   // refus : Failed(WouldOverwrite(conflit)) -> l'interface propose
//!   //   « Mettre de côté et basculer » : même demande avec LocalChanges::Stash
//!   //   « Forcer la bascule »          : vcs.switch_discarding_changes(&repo, cible, conflit)
//! vcs.rename_branch(&repo, old, new);                         // -> BranchRenamed { upstream }
//! vcs.delete_branch(&repo, name);   // -> BranchDeleted, ou Failed(BranchNotMerged { commits, subjects })
//! vcs.force_delete_branch(&repo, name);                       // destructif, après confirmation
//! vcs.restore_branch(&repo, name, tip);                       // « Restaurer »
//! vcs.check_detached_commits(&repo);                          // -> DetachedCommits { count }
//! // Distant (aucune saisie d'identifiants ; échec -> AuthenticationFailed)
//! vcs.fetch(&repo, FetchRequest::default());                  // --all --prune -> Fetched
//! vcs.update_project(&repo, UpdateRequest { method, local_changes: LocalChanges::Stash }); // -> Updated
//! vcs.pull(&repo, PullRequest { remote, branch, mode, local_changes }); // -> Pulled
//! vcs.fast_forward_branch(&repo, name);                       // -> BranchFastForwarded
//! vcs.load_push_preview(&repo, None, None);                   // -> PushPreview { outgoing, new_branch, protected }
//! vcs.push(&repo, PushRequest { target, set_upstream, follow_tags, force: PushForce::No });
//!   // refus : Failed(PushRejected { rejection: NonFastForward | StaleLease | RemoteRejected … })
//! vcs.delete_remote_branch(&repo, remote, branch);            // destructif, refusé si protégée
//! // Opérations (un arrêt sur conflit n'est pas un échec : finished == false)
//! vcs.merge(&repo, MergeRequest { revision, ..Default::default() }); // -> OperationRan
//! vcs.rebase(&repo, RebaseRequest { onto, branch: None, local_changes });
//! vcs.cherry_pick(&repo, commits, LocalChanges::Keep);  vcs.revert_commits(&repo, commits, LocalChanges::Keep);
//! vcs.continue_operation(&repo, None); vcs.skip_operation(&repo); vcs.abort_operation(&repo);
//! vcs.load_reset_preview(&repo, target);                      // -> ResetPreview { commits_leaving, unreachable, pushed }
//! vcs.reset(&repo, target, ResetMode::Mixed);  vcs.reset_hard(&repo, target, stash_first);
//! vcs.load_stashes(&repo);  vcs.restore_auto_stash(&repo, commit);
//! // Menus des fichiers
//! vcs.stage(&repo, paths); vcs.unstage(&repo, paths); vcs.discard_changes(&repo, paths);
//! vcs.move_path(&repo, from, to); vcs.add_to_gitignore(&repo, IgnoreRule::Extension("log".into()));
//! vcs.load_log(&repo, LogQuery::file_history(path));          // historique d'un fichier (--follow)
//! // Graphe : pur, page par page, dans le fil de l'interface (quelques µs par commit)
//! let mut graph = GraphBuilder::new();
//! let rows = graph.push_page(&page.commits);                   // GraphRow { lane, color, edges, width }
//! ```
//!
//! **Mise de côté automatique** ([`LocalChanges::Stash`]) : `git stash push
//! --include-untracked` avec un message qui commence par
//! [`AUTO_STASH_PREFIX`] (« JulieStorm : avant bascule de main vers x — date
//! UTC »), opération, puis `git stash apply` et suppression de l'entrée si la
//! restauration est propre. Sinon l'entrée est **gardée** et
//! [`AutoStash::restore`] le dit (`Conflicts`, `Pending` quand l'opération
//! s'est arrêtée, `Failed`). L'étagère maison prendra ce rôle plus tard.
//!
//! **Destructif, toujours nommé** : [`switch_discarding_changes`],
//! [`force_delete_branch`], [`delete_remote_branch`], [`reset_hard`],
//! [`discard_changes`], [`skip_operation`], [`accept_side`],
//! [`abort_operation`]. Ils ne prennent aucune copie de sécurité :
//! l'appelant fait la sienne avant (`decisions-v1.md` n° 3, dans
//! `julie-core`) à partir de [`OverwriteConflict::files`],
//! [`ResetPreview::local_changes`] ou de la liste qu'il a confirmée. Jamais de
//! `push --force` nu : seulement `--force-with-lease`, refusé sur les branches
//! protégées ([`VcsSettings::protected_branches`], défaut `main`, `master`).
//!
//! **Dans une tâche ou un test : les fonctions synchrones** ([`open_repository`],
//! [`status`], [`read_version`], [`load_diff`], [`commit`], [`log`],
//! [`commit_details`], [`conflict_versions`], [`resolve_with_content`],
//! [`mark_resolved`], [`accept_side`], [`unmerged_paths`],
//! [`continue_operation`], [`abort_operation`], et pour la v1
//! [`list_branches`], [`switch_branch`], [`fetch`], [`push`], [`merge`],
//! [`rebase`]…). Toutes **bloquantes**, elles prennent un [`Git`], un
//! [`Repository`] et un [`julie_core::CancelToken`] ; les opérations réseau
//! prennent en plus un rappel de progression. Les analyseurs de sortie
//! ([`parse_status`], [`parse_fetch_porcelain`], [`parse_overwrite`],
//! [`parse_progress_line`]) et le graphe ([`GraphBuilder`]) sont purs.
//!
//! # Chemins
//!
//! Chemins **absolus** partout dans l'API (écrits comme la racine donnée à
//! `open`, sans canonisation), plus le chemin Git (`repo_path`, relatif à la racine
//! du dépôt, séparé par `/`) dans les structures. Les commandes reçoivent des
//! chemins littéraux (`GIT_LITERAL_PATHSPECS=1`, sorties `-z`) : espaces, accents,
//! crochets et apostrophes passent tels quels.
//!
//! # Lancement de Git
//!
//! Un seul point d'entrée interne ([`Git`]) : `--no-pager`, `color.ui=false`,
//! `core.quotepath=false`, `log.showSignature=false` ; environnement `LC_ALL=C`,
//! `GIT_TERMINAL_PROMPT=0`, `GIT_EDITOR=:` (jamais d'éditeur), `GIT_OPTIONAL_LOCKS=0`
//! pour les lectures ; `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE` hérités
//! retirés. Crochets et signature ne sont **jamais** contournés. Chaque commande
//! est inscrite au [`CommandLog`] (500 dernières) et publiée en
//! [`VcsEvent::Command`].
//!
//! # Contenu des versions
//!
//! [`Content`] : `Missing`, `Blob { bytes, mode }`, `TooLarge { size }` (au-delà de
//! 20 Mio), `Submodule { commit }`, `Directory`. Les versions tirées de Git
//! passent par `git cat-file --filters` : octets tels que Git les écrirait sur le
//! disque (`autocrlf`, `.gitattributes`). Détection du binaire :
//! [`Content::is_binary`] (octet NUL dans les 8 premiers Kio). La fusion n'est pas
//! faite ici : `app` passe `base` / `ours` / `theirs` à `julie-diff` et rend le
//! résultat à [`Vcs::resolve_conflict`].
//!
//! # Rien de destructif en douce
//!
//! Pas de `reset --hard`, `clean`, `checkout -- <fichier>` ni `push --force`
//! implicites. Les fonctions qui écrasent du travail sont distinctes et le
//! disent dans leur documentation (liste plus haut). Une seule entorse aux
//! chemins littéraux : `git stash push|apply` tourne sans
//! `GIT_LITERAL_PATHSPECS` (sous chemins littéraux, Git laisse les non suivis
//! sur le disque après les avoir rangés).

mod branches;
mod commit;
mod conflict;
mod content;
mod error;
mod files;
mod git;
mod graph;
mod history;
mod operations;
mod remote;
mod repo;
mod service;
mod stash;
mod status;

pub use branches::{
    branch_name_collision, branch_tree, check_branch_name, commits_only_on_detached_head,
    create_branch, delete_branch, force_delete_branch, is_ancestor, list_branches, list_remotes,
    rename_branch, restore_branch, suggest_branch_name, switch_branch, switch_discarding_changes,
    Branch, BranchCommit, BranchKind, BranchList, BranchNode, CreateBranchOutcome,
    CreateBranchRequest, DeleteBranchOutcome, RenameBranchOutcome, SwitchOutcome, SwitchRequest,
    SwitchTarget, Tracking, RECENT_BRANCHES,
};
pub use commit::{commit, CommitOutcome, CommitRequest};
pub use conflict::{
    abort_operation, accept_side, conflict_versions, continue_operation, mark_resolved,
    resolve_with_content, unmerged_paths, ConflictSide, ConflictVersions, ContinueOutcome,
    SideResolution,
};
pub use content::{
    is_binary, load_diff, read_version, Content, DiffRequest, DiffSides, FileMode, Revision, Stage,
    MAX_CONTENT_SIZE,
};
pub use error::{BranchNameProblem, OverwriteConflict, PushRejection};
pub use error::{Result, VcsError};
pub use files::{
    add_to_gitignore, discard_changes, ignore_line, move_path, stage, unstage, DiscardOutcome,
    IgnoreAdded, IgnoreRule,
};
pub use git::{is_authentication_failure, is_protected, IntegrationMethod};
pub use git::{CommandLog, CommandOutcome, CommandRecord, CommandResult, Git, VcsSettings};
pub use graph::{graph_rows, EdgeHalf, GraphBuilder, GraphEdge, GraphRow};
pub use history::LogOrder;
pub use history::{
    commit_details, log, CommitDetails, CommitFile, CommitSummary, LogPage, LogQuery, Person,
    RefKind, RefLabel, DEFAULT_PAGE_SIZE,
};
pub use operations::{
    cherry_pick, merge, rebase, reset, reset_hard, reset_preview, revert_commits, skip_operation,
    FastForward, MergeRequest, OperationOutcome, RebaseRequest, ResetMode, ResetOutcome,
    ResetPreview,
};
pub use remote::{
    classify_rejection, delete_remote_branch, fast_forward_branch, fetch, parse_fetch_porcelain,
    parse_progress_line, pull, push, push_preview, push_target, update_project, FetchOutcome,
    FetchRequest, ProgressParser, ProgressPhase, PullMode, PullRequest, PushForce, PushOutcome,
    PushPreview, PushRequest, PushTarget, PushedKind, RefUpdate, RefUpdateKind,
    RemoteBranchDeleted, RemoteProgress, UpdateRequest,
};
pub use repo::{open_repository, Operation, OperationKind, Repository};
pub use service::{is_relevant_change, Action, Failure, Vcs, VcsEvent, STATUS_DEBOUNCE};
pub use stash::{
    list_stashes, parse_overwrite, restore_auto_stash, AutoStash, LocalChanges, StashEntry,
    StashReason, StashRestore, AUTO_STASH_PREFIX,
};
pub use status::{
    parse_status, status, Change, ConflictKind, FileKind, FileStatus, Group, Head, IndexState,
    Status, Upstream,
};
