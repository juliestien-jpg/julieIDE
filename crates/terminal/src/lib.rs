//! Terminal intégré : pseudo-console Windows (ConPTY), émulation VT, sessions
//! cmd / Git Bash / PowerShell. Propriétaire : dev-terminal.
//!
//! Ne dépend que de `julie-core` (plus `alacritty_terminal` pour l'émulation et
//! `windows-sys`) : ni egui, ni autre crate métier. Tout se teste sans fenêtre.
//!
//! # Vue d'ensemble
//!
//! ```text
//!  app (fil de l'interface)                 julie-terminal
//!  ───────────────────────                  ──────────────
//!  Terminals::open(options) ─────────────▶  tâche « terminal.session » :
//!                                             lance l'interpréteur dans une
//!                                             pseudo-console + un « job »,
//!                                             lit la sortie, l'analyse
//!  bus.drain() ◀── TerminalEvent::Output ──   (alacritty_terminal), publie
//!  session.snapshot(&mut screen)              par lots (un Output en attente
//!    (copie de l'écran sous verrou)           au plus par session)
//!  session.send_text / send_key / paste ──▶ fil de contrôle : écriture,
//!  session.resize(cols, lignes, …)           redimensionnement, fin de
//!  session.close()                            l'interpréteur, fermeture
//! ```
//!
//! # Utilisation
//!
//! ```no_run
//! use julie_core::{bus::Bus, Tasks};
//! use julie_terminal::{
//!     detect_profiles, Key, Modifiers, Screen, SessionOptions, TerminalEvent,
//!     TerminalSettings, Terminals,
//! };
//!
//! let bus = Bus::new();
//! let tasks = Tasks::new(bus.publisher());
//! let settings = TerminalSettings::default();            // section [terminal]
//! let terminals = Terminals::new(tasks, settings.clone());
//!
//! // Profils : `terminals.detect_profiles()` publie TerminalEvent::Profiles ;
//! // la version synchrone (accès disque) sert aux tests.
//! let profiles = detect_profiles(&settings);
//! let profile = profiles.default_profile().unwrap().clone();  // Git Bash s'il est installé
//!
//! let session = terminals.open(SessionOptions::new(profile, r"C:\projet"));
//! session.resize(120, 30, 8, 17);                         // cellules, pixels d'une cellule
//! session.send_text("echo é");
//! session.send_key(Key::Enter, Modifiers::NONE);
//!
//! let mut screen = Screen::default();
//! for event in bus.drain() {
//!     if let julie_core::Event::Domain(event) = event {
//!         if let Some(TerminalEvent::Output { session: id }) = event.get::<TerminalEvent>() {
//!             if *id == session.id() {
//!                 session.snapshot(&mut screen);          // puis dessiner `screen`
//!             }
//!         }
//!     }
//! }
//! session.close();   // tue l'interpréteur et tous ses descendants ; Closed suit
//! ```
//!
//! # API publique
//!
//! - [`Terminals`] (service, clonable) : `new(tasks, settings)`,
//!   `detect_profiles() -> TaskHandle` (`terminal.profiles`), `open(SessionOptions)
//!   -> Session` (non bloquant).
//! - [`Session`] (possédée par l'onglet ; la lâcher la ferme) :
//!   - état : `id()`, `profile()`, `status() -> SessionStatus` (`Starting`,
//!     `Running { pid }`, `Exited { code }`, `Closed`, `Failed(erreur)`),
//!     `title()`, `generation()` ;
//!   - affichage : `snapshot(&mut Screen) -> génération`, `resize(colonnes,
//!     lignes, largeur_px, hauteur_px)`, `size()`, `scroll(ScrollAction)`,
//!     `wheel(lignes)` (flèches dans l'écran alternatif) ;
//!   - saisie : `send_text(&str)`, `send_key(Key, Modifiers) -> bool`,
//!     `paste(&str)` (collage encadré si demandé), `write(octets)` ;
//!   - sélection : `start_selection(ScreenPoint, SelectionKind)`,
//!     `extend_selection`, `clear_selection`, `selection_text()`,
//!     `has_selection()` ;
//!   - fin : `running_programs()` (pour « Un programme tourne encore… »),
//!     `close()` (non bloquant), `close_and_wait(délai)` et `wait(délai)`
//!     (**bloquants** : tests, fermeture de l'IDE).
//! - [`TerminalEvent`] (sous `Domain::Terminal`) : `Profiles`, `Started`,
//!   `Output` (coalescé), `TitleChanged`, `Bell`, `Clipboard` (OSC 52),
//!   `Exited { code }`, `Closed`, `Failed { error }`.
//! - [`Screen`] : `columns`, `lines`, `cells` ([`ScreenCell`] : `ch`,
//!   `zerowidth`, `fg`, `bg` ([`Color`] : `Foreground`, `Background`,
//!   `Indexed(0..=255)`, `Rgb`), `flags` ([`CellFlags`])), `cursor`,
//!   `display_offset`, `history_size`, `modes` ([`Modes`]), `row(l)`,
//!   `line_text(l)`, `text()`. [`xterm_rgb`] pour les couleurs 16–255.
//! - Saisie : [`Key`], [`Modifiers`], [`encode_key`], [`encode_paste`],
//!   [`control_byte`] (fonctions pures).
//! - [`Emulator`] : l'émulateur seul (octets → écran), sans processus.
//! - Profils : [`ProfileKind`] (`Cmd`, `GitBash`, `PowerShell`), [`Profile`],
//!   [`Profiles`], [`TerminalSettings`] (`[terminal]` : `default_profile`,
//!   `git_bash_path`, `cmd_path`, `scrollback`), [`detect_profiles`] (accès
//!   disque), [`profile`] (sans disque).
//! - Erreurs : [`TerminalError`].
//!
//! # Garanties
//!
//! - **Aucune lecture bloquante sur le fil de l'interface** : les méthodes de
//!   `Session` posent une commande dans un canal ou copient l'écran sous verrou.
//!   Seuls `close_and_wait` et `wait` attendent.
//! - **Aucun processus orphelin** : l'interpréteur est lancé suspendu, placé
//!   dans un objet « job » Windows (`KILL_ON_JOB_CLOSE`), puis relâché ; aucun
//!   descendant ne peut s'en échapper. Fermer une session tue tout le job puis
//!   ferme la pseudo-console (conhost s'arrête). Si l'IDE meurt, Windows ferme
//!   le job et tue l'arbre. Quand l'interpréteur se termine de lui-même, ce
//!   qu'il a laissé tourner est tué aussi.
//! - Environnement hérité de l'IDE, plus les variables du profil (Git Bash :
//!   `TERM=xterm-256color`, `COLORTERM=truecolor`, `CHERE_INVOKING=1`).
//! - Sortie de la pseudo-console : toujours UTF-8. Saisie : UTF-8.
//! - Historique borné (10 000 lignes par défaut) ; une nouvelle sortie ne
//!   déplace pas une vue remontée dans l'historique ; toute saisie la ramène en
//!   bas.

mod emulator;
mod error;
mod input;
mod profile;
#[cfg(windows)]
mod pty;
#[cfg(not(windows))]
#[path = "pty_unsupported.rs"]
mod pty;
mod screen;
mod session;

pub use emulator::{
    Emulator, ScreenPoint, ScrollAction, SelectionKind, Signal, DEFAULT_SCROLLBACK,
};
pub use error::TerminalError;
pub use input::{control_byte, encode_key, encode_paste, Key, Modifiers};
pub use profile::{
    detect_profiles, profile, Profile, ProfileKind, Profiles, TerminalSettings, DEFAULT_GIT_BASH,
};
pub use screen::{
    xterm_rgb, CellFlags, Color, CursorShape, Modes, Screen, ScreenCell, ScreenCursor,
};
pub use session::{Session, SessionId, SessionOptions, SessionStatus, TerminalEvent, Terminals};
