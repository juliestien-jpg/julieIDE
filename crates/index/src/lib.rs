//! Index des fichiers et recherche. Propriétaire : dev-indexation-recherche.
//!
//! Cette crate ne dépend que de `julie-core` : ni egui, ni autre crate métier.
//! Spécification : `docs/specs/recherche.md`. Contrat : `docs/contrats.md`,
//! section « `julie-index` ».
//!
//! Spécification de la navigation : `docs/specs/navigation.md`.
//!
//! # Périmètre
//!
//! - **Index des fichiers** ([`FileIndex`]) : parcours parallèle respectant
//!   `.gitignore` (même sans dépôt Git), `.git/info/exclude` et l'exclusion
//!   globale de Git ; `.git` et `.juliestorm` toujours exclus
//!   (`julie_core::project::ALWAYS_EXCLUDED`), plus `[index] excluded` (défaut
//!   `/target`) ; `vendor/` et `node_modules/` indexés mais marqués
//!   « bibliothèque » ; mise à jour incrémentale.
//! - **Aller au fichier** ([`FileIndex::find_files`]) et **recherche dans les
//!   fichiers** ([`Matcher`], [`search_files`], [`preview_file`]).
//! - **Index des symboles** ([`SymbolIndex`], v1) : déclarations PHP (espaces de
//!   noms, classes, interfaces, traits, énumérations et leurs cas, méthodes,
//!   propriétés y compris promues, constantes de classe, fonctions, constantes
//!   `const`/`define`) et JS/TS (fonctions, classes et méthodes, variables de
//!   premier niveau, imports, exports, réexports, interfaces, alias de types,
//!   énumérations), par tree-sitter ; incrémental ; onglets non enregistrés ;
//!   cache versionné `.juliestorm/index/symbols.bin`. Fichiers de plus de
//!   1 Mio (`SYMBOL_MAX_FILE_SIZE`) et `*.min.js` non indexés.
//! - **Navigation** ([`navigate`], v1) : ce qu'il y a sous une position —
//!   cibles d'« aller à la déclaration » (sûres ou devinées par le nom), la
//!   déclaration elle-même (→ usages), un fichier (chemin d'import), ou rien
//!   et pourquoi.
//! - **Usages** ([`find_usages`], v1) : en flux, sûrs / possibles (type
//!   inconnu), bibliothèques exclues par défaut, bilan de ce qui n'a pas été
//!   cherché ([`UsagesSummary`]).
//! - **Service** ([`IndexService`]) : tout ce qui précède en tâches de fond,
//!   annulation de la demande précédente du même genre, résultats en
//!   [`IndexEvent`] ; réglages changés à chaud ([`IndexService::set_settings`]).
//!
//! # Navigation, côté `app`
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use julie_core::{Bus, Project, Tasks, TextPosition};
//! # use julie_index::*;
//! # let bus = Bus::new();
//! # let index = IndexService::new(Tasks::new(bus.publisher()), Project::open("C:/p").unwrap(), IndexSettings::default());
//! # let (path, text) = (std::path::PathBuf::from("C:/p/a.php"), Arc::<str>::from(""));
//! // Ctrl+clic, Ctrl+B, survol avec Ctrl : même demande.
//! let task = index.navigate(NavigationRequest { path, text, position: TextPosition::new(10, 4) });
//! // IndexEvent::Navigation(result), `event.task == Some(task.id())` sinon périmé :
//! # let result: NavigationResult = unimplemented!();
//! match result.navigation {
//!     Navigation::Targets(t) if t.targets.len() == 1 && t.guess.is_none() => { /* sauter */ }
//!     Navigation::Targets(t) => { /* liste surgissante ; t.guess : titre « type inconnu » */ }
//!     Navigation::Declaration(d) => { index.find_usages(UsagesQuery::new(d)); }
//!     Navigation::OpenFile(path) => { /* ouvrir */ }
//!     Navigation::Nothing(reason) => { /* barre d'état selon la raison */ }
//! }
//! // Soulignement : `result.word` (None = rien de navigable).
//! // Usages : IndexEvent::Usages(lot)… puis IndexEvent::UsagesFinished(bilan).
//! // Onglet modifié : index.set_unsaved(chemin, Some(texte)) ; enregistré/fermé : None.
//! ```
//!
//! # Conventions
//!
//! - Chemins **absolus** (séparateurs du système) dans `path` ; chemins
//!   **relatifs** à la racine avec des `/` dans `relative` (affichage, masques,
//!   correspondance floue).
//! - Lignes et colonnes en base zéro, colonnes en **caractères**
//!   (`julie_core::TextPosition`), comme l'éditeur. Les positions de surlignage
//!   d'« aller au fichier » sont des indices de caractères dans `relative`.
//! - Le texte cherché est celui que verrait l'éditeur : décodé par
//!   `julie_core::file::decode`, lignes séparées par `\n`, `\r` final retiré.
//!
//! # Utilisation par `app`
//!
//! ```no_run
//! use julie_core::{Bus, Domain, Event, Project, Tasks};
//! use julie_index::{IndexEvent, IndexService, IndexSettings, TextQuery, UnsavedFiles};
//!
//! let bus = Bus::new();
//! let tasks = Tasks::new(bus.publisher());
//! let project = Project::open("C:/projet").unwrap();     // dans une tâche, en vrai
//! let index = IndexService::new(tasks, project, IndexSettings::default());
//!
//! index.scan();                                          // à l'ouverture du projet
//! let find = index.find_files("usct", vec![]);           // à chaque frappe
//! let mut query = TextQuery::new("function");
//! query.whole_word = true;
//! query.file_mask = "*.php".into();
//! let search = index.search_text(query, UnsavedFiles::new()); // Err : motif invalide
//!
//! for event in bus.drain() {
//!     if let Event::Domain(event) = event {
//!         if event.domain != Domain::Index { continue; }
//!         let task = event.task;                         // écarter les demandes périmées
//!         match event.get::<IndexEvent>() {
//!             Some(IndexEvent::Files(results)) => { /* liste d'« aller au fichier » */ }
//!             Some(IndexEvent::TextMatches(files)) => { /* ajouter au fil de l'eau */ }
//!             Some(IndexEvent::TextSearchFinished(summary)) => { /* compteur final */ }
//!             _ => {}
//!         }
//!     }
//! }
//! ```
//!
//! Les fonctions synchrones ([`FileIndex::scan`], [`FileIndex::update`],
//! [`FileIndex::find_files`], [`files_to_search`], [`search_files`],
//! [`preview_file`]) font le travail sans bus : ce sont elles que testent les
//! tests, et une tâche écrite par `app` peut les appeler directement.

mod error;
mod files;
mod fuzzy;
mod service;
mod settings;
mod symbols;
mod text;

pub use error::{IndexError, Result};
pub use files::{FileEntry, FileIndex, UpdateSummary};
pub use fuzzy::{split_location, FileMatch, FileResults, MatchKind, DEFAULT_LIMIT, RECENT_LIMIT};
pub use service::{IndexEvent, IndexService, ScanSummary};
pub use settings::IndexSettings;
pub use symbols::{
    find_usages, navigate, Declaration, Guess, Navigation, NavigationRequest, NavigationResult,
    NoTarget, SymbolBuildSummary, SymbolIndex, SymbolKind, SymbolLanguage, Targets, Usage,
    UsageCertainty, UsagesQuery, UsagesSummary, SYMBOL_CACHE_FILE, SYMBOL_MAX_FILE_SIZE,
};
pub use text::{
    files_to_search, preview_file, preview_text, search_files, FileMask, FileTextMatches,
    LineMatch, Matcher, Preview, PreviewLine, SearchFile, SearchOptions, SearchScope, TextQuery,
    TextSearchSummary, UnsavedFiles, DEFAULT_MAX_MATCHES,
};
