//! Composant éditeur : affiche et édite un [`julie_text::Document`]. Propriétaire : dev-editeur.
//!
//! Le composant ne connaît ni Git, ni PHPUnit, ni la recherche : il reçoit des
//! [`Decorations`] (fonds de lignes et de plages, marques de gouttière,
//! séparateurs, mot navigable souligné) et rend des [`EditorIntent`] (clic sur une marque,
//! Ctrl+clic ou Ctrl+B, survol avec Ctrl : l'éditeur ne sait pas ce que l'hôte en fait). Les couleurs
//! viennent d'un [`EditorTheme`], jamais du code.
//!
//! # Utilisation
//!
//! ```ignore
//! // Une fois : le document vient de julie_text (TextEvent::Loaded ou Document::new).
//! let mut document = julie_text::Document::from_loaded(loaded);
//! document.enable_syntax(Some(waker));               // waker = request_repaint
//! let mut view = EditorView::new(&path);             // l'identifiant doit être stable
//!
//! // À chaque image :
//! let output = view.show(ui, &mut document, &decorations);
//! if output.changed { /* marque « modifié » de l'onglet, diff à recalculer… */ }
//! for intent in output.intents { /* GutterMarkClicked { line, id } */ }
//! ```
//!
//! - **Seules les lignes visibles** sont mises en page ; une ligne de plus de
//!   2 000 caractères ne l'est que sur sa partie visible (plus une marge).
//! - Saisie : frappe (accents, touches mortes, AltGr), composition IME (texte en
//!   cours affiché souligné au curseur, inséré à la validation), copier / couper /
//!   coller avec le presse-papiers du système (sans sélection : la ligne entière).
//! - Raccourcis de l'éditeur (voir `docs/specs/raccourcis.md`) : flèches (Ctrl :
//!   par mot ; Maj : sélection), Origine / Fin (intelligent ; Ctrl : document),
//!   Page préc. / suiv., Ctrl+Haut / Bas (défile), Retour arrière / Suppr (Ctrl : par
//!   mot), Entrée (indentation reprise), Tab / Maj+Tab, Ctrl+Z / Ctrl+Maj+Z,
//!   **Ctrl+Y supprime la ligne**, Ctrl+D duplique, Ctrl+/ commente, Alt+Maj+Haut /
//!   Bas déplace les lignes, Ctrl+A, Échap (réduit la sélection). Ctrl+S et les
//!   raccourcis globaux restent à l'hôte.
//! - Souris : clic, Maj+clic, double-clic (mot), triple-clic (ligne), glisser (par
//!   caractère, mot ou ligne selon le clic de départ, avec défilement automatique),
//!   Alt+clic ajoute un curseur, clic dans les numéros sélectionne la ligne,
//!   molette (Maj : horizontale), barres de défilement.
//! - [`EditorOptions::read_only`] : ni frappe ni collage, sélection et copie
//!   possibles. Un document lui-même en lecture seule
//!   (`Document::set_read_only`) refuse aussi toute modification.
//!
//! # Volets côte à côte (diff, fusion)
//!
//! L'interface pose deux ou trois éditeurs avec [`split_panes`] (qui rend aussi
//! les gouttières entre volets), les affiche par [`EditorView::show_in_rect`],
//! puis appelle [`sync_scroll`] : le volet que l'utilisatrice a fait défiler
//! (`EditorOutput::scrolled`) entraîne les autres par une fonction de
//! correspondance des lignes fournie par l'appelant. La géométrie de chaque volet
//! ([`EditorLayout`] : `line_top`, `lines_y_range`, `visible_lines`…) permet de
//! dessiner les connecteurs dans les gouttières. Position imposée de l'extérieur :
//! [`EditorView::set_top_line`], [`EditorView::set_scroll_y`],
//! [`EditorView::center_line`].

// API publique du composant : une partie (décorations, géométrie, intentions)
// n'est utilisée qu'à partir du lot d'assemblage (diff, fusion, recherche).
#![allow(dead_code, unused_imports)]

mod decorations;
mod layout;
mod theme;
mod view;

pub use decorations::{Decorations, GutterMark, LineDecoration, MarkShape, RangeDecoration, Tone};
pub use layout::{split_panes, sync_scroll, EditorLayout};
pub use theme::EditorTheme;
pub use view::{EditorIntent, EditorOptions, EditorOutput, EditorView};
