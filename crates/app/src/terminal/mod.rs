//! Terminal intégré côté interface : le composant qui affiche une
//! [`julie_terminal::Session`] et le branchement dans la fenêtre d'outils du
//! bas. Propriétaire : dev-terminal (l'habillage de `panel.rs` passera à
//! dev-interface).
//!
//! # Composant : [`TerminalView`]
//!
//! ```ignore
//! let mut view = TerminalView::new(session.id());     // identifiant stable
//! view.request_focus();
//! // À chaque image :
//! let output = view.show(ui, &session);               // ou show_in_rect(ui, rect, &session)
//! for intent in output.intents { /* TerminalIntent::OpenFile { path, line, column } */ }
//! ```
//!
//! - Taille : la place disponible, divisée par la cellule de la police à chasse
//!   fixe, est donnée à la session à chaque image (`Session::resize` ne fait
//!   rien si elle n'a pas changé ; sinon émulateur et pseudo-console suivent).
//! - Dessin : l'écran n'est recopié de la session (`Session::snapshot`) que si
//!   sa génération a changé ; seules les lignes visibles existent. Fonds par
//!   plages de même couleur, texte par plages de même style ; un caractère non
//!   ASCII ou large est placé seul à sa colonne. Couleurs du [`TerminalTheme`]
//!   (16 ANSI, défaut, curseur, sélection ; 256 et 24 bits calculées) ; gras
//!   (éclaircit 0–7, tracé doublé), italique, souligné, barré, inverse, atténué,
//!   caché ; curseur bloc / barre / souligné, creux sans le focus.
//! - Clavier (focus) : tout va au shell — texte (accents, touches mortes,
//!   AltGr, IME), Entrée, Tab, Échap, flèches, F1–F12, Ctrl+lettre, Alt+lettre —
//!   sauf Alt+F12, Alt+0–9, Ctrl+Maj+N/F/T et Ctrl+F4 laissés à l'IDE (voir
//!   `docs/specs/raccourcis.md`). Ctrl+C copie s'il y a une sélection, sinon
//!   interrompt ; Ctrl+V et Maj+Inser collent (encadré si le shell l'a
//!   demandé) ; Ctrl+Maj+C / Ctrl+Maj+V copient / collent toujours ;
//!   Maj+Page préc. / suiv. défilent l'historique.
//! - Souris : glisser = sélection par caractère, double-clic = mot, triple-clic
//!   = ligne, Maj+clic étend ; glisser au-delà du bord fait défiler ; molette =
//!   historique (flèches envoyées à less / vim dans l'écran alternatif) ; clic
//!   droit = Copier / Coller / Effacer l'historique ; Ctrl+clic sur un
//!   `chemin:ligne` = [`TerminalIntent::OpenFile`].
//!
//! # Branchement : [`TerminalPanel`]
//!
//! La coquille (`main.rs`) n'appelle que : `TerminalPanel::new(tasks, cwd,
//! réglages)`, `on_event(&TerminalEvent)` en vidant le bus, `ui_global(ui)` à
//! chaque image (confirmation à la fermeture de l'IDE, presse-papiers OSC 52),
//! `show(ui)` dans l'onglet Terminal, `has_focus(ctx)` (pour laisser Ctrl+S au
//! shell) et `focus()` (Alt+F12). Les onglets de sessions, le « + » et son
//! menu cmd / Git Bash, le renommage (double-clic), la fermeture avec
//! confirmation et « Relancer » vivent dans `panel.rs`.

// API publique du composant : une partie (géométrie rendue, thème imposé,
// instantané) ne sert qu'à partir de l'habillage par dev-interface.
#![allow(dead_code, unused_imports)]

mod keys;
mod panel;
mod theme;
mod view;

pub use panel::{resolve_path, TerminalPanel};
pub use theme::TerminalTheme;
pub use view::{link_in_line, TerminalIntent, TerminalOutput, TerminalView};
