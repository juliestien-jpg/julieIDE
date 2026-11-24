//! Git piloté par la ligne de commande : état, diff, commit, journal, conflits.
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
//! écritures sont sérialisées entre elles.
//!
//! **Dans une tâche ou un test : les fonctions synchrones** ([`open_repository`],
//! [`status`], [`read_version`], [`load_diff`], [`commit`], [`log`],
//! [`commit_details`], [`conflict_versions`], [`resolve_with_content`],
//! [`mark_resolved`], [`accept_side`], [`unmerged_paths`],
//! [`continue_operation`], [`abort_operation`]). Toutes **bloquantes**, elles
//! prennent un [`Git`], un [`Repository`] et un [`julie_core::CancelToken`].
//! Les analyseurs de sortie ([`parse_status`]) sont purs.
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
//! Pas de `reset --hard`, `clean`, `checkout -- <fichier>` ni `push --force`.
//! Seules deux fonctions écrasent du travail, et le disent : [`accept_side`]
//! (garde un côté entier, peut supprimer le fichier) et [`abort_operation`]
//! (défait la fusion en cours).

mod commit;
mod conflict;
mod content;
mod error;
mod git;
mod history;
mod repo;
mod service;
mod status;

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
pub use error::{Result, VcsError};
pub use git::{CommandLog, CommandOutcome, CommandRecord, CommandResult, Git, VcsSettings};
pub use history::{
    commit_details, log, CommitDetails, CommitFile, CommitSummary, LogPage, LogQuery, Person,
    RefKind, RefLabel, DEFAULT_PAGE_SIZE,
};
pub use repo::{open_repository, Operation, OperationKind, Repository};
pub use service::{is_relevant_change, Action, Failure, Vcs, VcsEvent, STATUS_DEBOUNCE};
pub use status::{
    parse_status, status, Change, ConflictKind, FileKind, FileStatus, Group, Head, Status, Upstream,
};
