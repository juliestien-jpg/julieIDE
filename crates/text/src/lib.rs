//! Tampon de texte (corde), édition, annulation et syntaxe tree-sitter. Propriétaire : dev-editeur.
//!
//! Cette crate ne dépend que de `julie-core` : ni egui, ni autre crate métier.
//! Tout se teste sans fenêtre.
//!
//! # Vue d'ensemble
//!
//! | Type | Rôle |
//! |---|---|
//! | [`Buffer`] | La corde (`ropey`), sa version, l'historique d'annulation groupé, l'état « modifié », les ancres ([`Buffer::map_offset_since`]). |
//! | [`Selections`] / [`Selection`] | Curseurs multiples (modèle prêt, la v0 n'en montre qu'un). Positions en **caractères**. |
//! | [`Document`] | Un tampon + ses sélections + son fichier (chemin, [`TextFormat`](julie_core::file::TextFormat), [`DiskStamp`]) + sa coloration. Toutes les commandes d'édition. |
//! | [`SyntaxSession`] | Coloration tree-sitter incrémentale dans un fil de fond, par document. |
//! | [`Language`], [`HighlightKind`], [`HighlightSpan`] | Langages reconnus, catégories de coloration (pas de couleurs : le thème est côté interface), plages par ligne. |
//! | [`TextEvent`], [`spawn_load`], [`spawn_save`], [`spawn_check_disk`] | Disque par tâches de fond (`Domain::Text`). |
//!
//! # Positions
//!
//! - Positions en **caractères** (valeurs scalaires Unicode) dans le texte :
//!   `usize` pour un décalage, [`TextPosition`](julie_core::TextPosition)
//!   (ligne, colonne) aux frontières. Conversions : [`Buffer::offset_to_position`],
//!   [`Buffer::position_to_offset`].
//! - Lignes séparées par `\n`, `\r\n` ou `\r` ; [`Buffer::line`] rend le contenu
//!   **sans** sa fin de ligne. Les déplacements franchissent `\r\n` d'un coup et
//!   les graphèmes (lettre + accent combinant, drapeaux, émojis composés) entiers.
//! - Les plages colorées sont en **octets** du contenu de la ligne
//!   ([`HighlightSpan`]) : c'est ce que demande la mise en page du texte.
//!
//! # Fichiers
//!
//! ```no_run
//! # use julie_text::*;
//! # fn demo(tasks: &julie_core::Tasks, path: std::path::PathBuf) -> TextResult<()> {
//! // Dans une tâche (accès disque) :
//! let mut document = Document::open(&path)?;          // ou spawn_load -> TextEvent::Loaded
//! // Dans le fil de l'interface :
//! document.insert_text("é");
//! let request = document.save_request()?;             // copie du texte + format
//! spawn_save(tasks, request);                         // -> TextEvent::Saved(outcome)
//! // à réception : document.mark_saved(&outcome);
//! # Ok(()) }
//! ```
//!
//! - Encodage, BOM et fins de ligne passent par `julie_core::file` et sont
//!   rendus à l'identique : en dehors des lignes touchées, le fichier enregistré
//!   est identique octet pour octet. Pour un fichier homogène le texte en mémoire
//!   n'a que des `\n` ; pour un fichier mélangé (`LineEnding::Mixed`) il est
//!   exact, et une ligne nouvelle (Entrée, collage) prend la fin de ligne la plus
//!   fréquente du fichier.
//! - [`load_file`] refuse un fichier binaire (NUL dans les 8 premiers Kio,
//!   [`TextError::Binary`]) ou de plus de 50 Mio ([`TextError::TooLarge`]) ;
//!   au-delà de 5 Mio le document est ouvert en texte brut. Un fichier
//!   Windows-1252 contenant un octet non attribué s'ouvre en lecture seule
//!   ([`ReadOnlyReason::UndefinedBytes`]).
//! - « Modifié » : [`Document::is_modified`] compare l'état de l'historique à
//!   celui enregistré ; annuler jusqu'à l'état enregistré retire la marque.
//! - « Changé sur le disque » : [`Document::disk_stamp`] (date et taille lues au
//!   chargement et après chaque enregistrement) ; [`check_disk`] /
//!   [`spawn_check_disk`] le comparent au disque. Recharger :
//!   [`Document::reload`] (historique vidé, curseur gardé sur sa ligne).
//!
//! # Annuler / rétablir
//!
//! Une frappe continue forme une seule étape ; l'étape se clôt à un déplacement
//! du curseur, un saut de ligne, un collage, une pause de plus d'une seconde
//! ([`GROUP_PAUSE`]) et à l'enregistrement. Annuler replace les sélections
//! d'avant l'étape.
//!
//! # Coloration
//!
//! ```no_run
//! # use julie_text::*;
//! # let mut document = Document::new("");
//! # let waker: Waker = std::sync::Arc::new(|| {});
//! document.enable_syntax(Some(waker));     // waker : ctx.request_repaint()
//! // une fois par image :
//! document.poll_syntax();
//! for span in document.highlights(0) { /* span.kind -> couleur du thème */ }
//! ```
//!
//! - Langages : PHP (avec le HTML hors des balises, et ses `<script>`/`<style>`),
//!   HTML (avec JS et CSS), CSS, JavaScript (JSX compris), TypeScript, TSX.
//!   [`Language::from_path`] choisit d'après l'extension.
//! - Analyse incrémentale (les arbres sont gardés et reçoivent chaque
//!   modification), toujours dans le fil de fond du document. En attendant le
//!   résultat, les plages affichées sont recalées sur les lignes ajoutées ou
//!   retirées : pas de clignotement.
//! - Les arbres de la dernière analyse sont lisibles par
//!   [`SyntaxSession::snapshot`] ([`SyntaxSnapshot`], clonage en temps constant),
//!   pour l'extraction de symboles d'un fichier ouvert.
//! - [`highlight_text`] colore un texte d'un coup, sans fil (tests, aperçus).

mod buffer;
mod document;
mod error;
pub mod movement;
mod selection;
mod service;
mod session;
mod syntax;

pub use buffer::{Bias, Buffer, ByteEdit, BytePoint, Change, EditKind, GROUP_PAUSE};
pub use document::{
    check_disk, load_file, DiskStamp, DiskState, Document, IndentStyle, LoadedFile, Motion,
    ReadOnlyReason, SaveOutcome, SaveRequest, MAX_HIGHLIGHT_SIZE, MAX_OPEN_SIZE,
};
pub use error::{TextError, TextResult};
pub use movement::TAB_WIDTH;
pub use selection::{Selection, Selections};
pub use service::{spawn_check_disk, spawn_load, spawn_save, TextEvent};
pub use session::{SyntaxSession, Waker};
pub use syntax::{
    highlight_text, HighlightKind, HighlightSpan, Highlighter, InjectedTree, Language,
    LineHighlights, SyntaxSnapshot,
};

/// La version de tree-sitter utilisée pour les arbres de [`SyntaxSnapshot`].
pub use tree_sitter;
