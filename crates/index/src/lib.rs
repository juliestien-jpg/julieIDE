//! Index des fichiers et recherche. Propriétaire : dev-indexation-recherche.
//!
//! Cette crate ne dépend que de `julie-core` : ni egui, ni autre crate métier.
//! Spécification : `docs/specs/recherche.md`. Contrat : `docs/contrats.md`,
//! section « `julie-index` ».
//!
//! # Périmètre (v0)
//!
//! - **Index des fichiers** ([`FileIndex`]) : parcours parallèle respectant
//!   `.gitignore` (même sans dépôt Git), `.git/info/exclude` et l'exclusion
//!   globale de Git ; exclusions des réglages (`.git`, `.juliestorm`, `/target`) ;
//!   `vendor/` et `node_modules/` indexés mais marqués « bibliothèque » ; mise à
//!   jour incrémentale à partir d'une liste de chemins changés.
//! - **Aller au fichier** ([`FileIndex::find_files`]) : correspondance floue,
//!   classement de la spécification, suffixe `:ligne[:colonne]`, récents en tête
//!   quand la requête est vide, positions des caractères appariés.
//! - **Recherche dans les fichiers** ([`Matcher`], [`search_files`]) : texte ou
//!   regex, casse, mot entier, masque, portée, contenus non enregistrés, binaires
//!   et fichiers de plus de 10 Mio ignorés, Windows-1252 décodé ; aperçu avec
//!   contexte ([`preview_file`]).
//! - **Service** ([`IndexService`]) : tout ce qui précède en tâches de fond, avec
//!   annulation de la demande précédente, résultats publiés en [`IndexEvent`].
//!
//! Hors v0 : symboles, recherche partout, remplacement, définition, usages,
//! cache disque.
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
mod text;

pub use error::{IndexError, Result};
pub use files::{FileEntry, FileIndex, UpdateSummary};
pub use fuzzy::{split_location, FileMatch, FileResults, MatchKind, DEFAULT_LIMIT, RECENT_LIMIT};
pub use service::{IndexEvent, IndexService, ScanSummary};
pub use settings::IndexSettings;
pub use text::{
    files_to_search, preview_file, preview_text, search_files, FileMask, FileTextMatches,
    LineMatch, Matcher, Preview, PreviewLine, SearchFile, SearchOptions, SearchScope, TextQuery,
    TextSearchSummary, UnsavedFiles, DEFAULT_MAX_MATCHES,
};
