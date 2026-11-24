//! Émulation VT : `alacritty_terminal` (analyseur `vte`, grille, historique,
//! écran alternatif, sélection) derrière une interface à nous. Aucune
//! entrée-sortie : on lui donne des octets, on lit l'écran. Testable seul.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event as AlacrittyEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AnsiColor, CursorShape as AnsiCursorShape, NamedColor, Processor, Rgb,
};

use crate::screen::{CellFlags, Color, CursorShape, Modes, Screen, ScreenCell, ScreenCursor};

/// Historique par défaut (lignes), comme demandé par la spécification.
pub const DEFAULT_SCROLLBACK: usize = 10_000;

/// Ce que l'émulateur demande à son hôte pendant l'analyse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Réponse à renvoyer au programme (position du curseur, attributs…).
    Reply(Vec<u8>),
    /// Titre fixé par le programme (OSC 0/2) ; `None` : retour au titre par défaut.
    Title(Option<String>),
    /// Le programme demande à mettre ce texte dans le presse-papiers (OSC 52).
    Clipboard(String),
    Bell,
}

/// Granularité d'une sélection à la souris.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// Caractère par caractère (glisser).
    Simple,
    /// Mot (double-clic).
    Word,
    /// Ligne entière (triple-clic).
    Line,
}

/// Un point de l'écran affiché, pour la sélection. `right_half` : le pointeur
/// est dans la moitié droite de la cellule (la cellule est alors incluse quand
/// on sélectionne vers la droite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenPoint {
    pub line: usize,
    pub column: usize,
    pub right_half: bool,
}

/// Défilement de la vue dans l'historique.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAction {
    /// Lignes vers le haut (positif, vers l'historique) ou le bas (négatif).
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Clone)]
struct Listener {
    signals: Arc<Mutex<Vec<Signal>>>,
    window: Arc<Mutex<WindowSize>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: AlacrittyEvent) {
        let signal = match event {
            AlacrittyEvent::PtyWrite(text) => Signal::Reply(text.into_bytes()),
            AlacrittyEvent::Title(title) => Signal::Title(Some(title)),
            AlacrittyEvent::ResetTitle => Signal::Title(None),
            AlacrittyEvent::ClipboardStore(_, text) => Signal::Clipboard(text),
            AlacrittyEvent::Bell => Signal::Bell,
            AlacrittyEvent::ColorRequest(index, format) => {
                let (r, g, b) = default_rgb(index);
                Signal::Reply(format(Rgb { r, g, b }).into_bytes())
            }
            AlacrittyEvent::TextAreaSizeRequest(format) => {
                let size = *self.window.lock().unwrap_or_else(|e| e.into_inner());
                Signal::Reply(format(size).into_bytes())
            }
            _ => return,
        };
        self.signals
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(signal);
    }
}

/// Couleur rendue à un programme qui interroge la palette (OSC 4/10/11 avec `?`).
fn default_rgb(index: usize) -> (u8, u8, u8) {
    match index {
        0..=255 => crate::screen::xterm_rgb(index as u8),
        257 => (0, 0, 0),
        _ => (229, 229, 229),
    }
}

#[derive(Debug, Clone, Copy)]
struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

/// Un terminal émulé : grille, historique, modes, sélection.
pub struct Emulator {
    term: Term<Listener>,
    processor: Processor,
    signals: Arc<Mutex<Vec<Signal>>>,
    window: Arc<Mutex<WindowSize>>,
}

impl std::fmt::Debug for Emulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emulator")
            .field("columns", &self.columns())
            .field("lines", &self.lines())
            .finish()
    }
}

impl Emulator {
    /// `columns × lines` cellules (au moins 2 × 1), `scrollback` lignes d'historique.
    pub fn new(columns: usize, lines: usize, scrollback: usize) -> Self {
        let size = Size {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        let signals = Arc::new(Mutex::new(Vec::new()));
        let window = Arc::new(Mutex::new(WindowSize {
            num_lines: size.lines as u16,
            num_cols: size.columns as u16,
            cell_width: 8,
            cell_height: 16,
        }));
        let config = Config {
            scrolling_history: scrollback,
            kitty_keyboard: false,
            osc52: Osc52::OnlyCopy,
            ..Config::default()
        };
        let listener = Listener {
            signals: signals.clone(),
            window: window.clone(),
        };
        Self {
            term: Term::new(config, &size, listener),
            processor: Processor::new(),
            signals,
            window,
        }
    }

    pub fn columns(&self) -> usize {
        self.term.columns()
    }

    pub fn lines(&self) -> usize {
        self.term.screen_lines()
    }

    /// Analyse des octets venus du programme ; rend ce qu'il demande à l'hôte.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Signal> {
        self.processor.advance(&mut self.term, bytes);
        self.take_signals()
    }

    fn take_signals(&mut self) -> Vec<Signal> {
        std::mem::take(&mut *self.signals.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Fin d'une mise à jour synchronisée restée ouverte trop longtemps
    /// (`ESC [ ? 2026 h` sans fin). Rend `true` si l'écran a changé.
    pub fn flush_sync_timeout(&mut self, now: Instant) -> bool {
        match self.processor.sync_timeout().sync_timeout() {
            Some(deadline) if deadline <= now => {
                self.processor.stop_sync(&mut self.term);
                true
            }
            _ => false,
        }
    }

    /// Nouvelle taille en cellules ; `cell_width`/`cell_height` en pixels
    /// (seulement pour répondre aux programmes qui les demandent).
    pub fn resize(&mut self, columns: usize, lines: usize, cell_width: u16, cell_height: u16) {
        let size = Size {
            columns: columns.max(2),
            lines: lines.max(1),
        };
        *self.window.lock().unwrap_or_else(|e| e.into_inner()) = WindowSize {
            num_lines: size.lines as u16,
            num_cols: size.columns as u16,
            cell_width: cell_width.max(1),
            cell_height: cell_height.max(1),
        };
        self.term.resize(size);
    }

    pub fn modes(&self) -> Modes {
        let mode = *self.term.mode();
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            mouse: mode.intersects(TermMode::MOUSE_MODE),
            show_cursor: mode.contains(TermMode::SHOW_CURSOR),
        }
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn scroll(&mut self, action: ScrollAction) {
        let scroll = match action {
            ScrollAction::Lines(n) => Scroll::Delta(n),
            ScrollAction::PageUp => Scroll::PageUp,
            ScrollAction::PageDown => Scroll::PageDown,
            ScrollAction::Top => Scroll::Top,
            ScrollAction::Bottom => Scroll::Bottom,
        };
        self.term.scroll_display(scroll);
    }

    fn grid_point(&self, point: ScreenPoint) -> (Point, Side) {
        let line =
            point.line.min(self.lines().saturating_sub(1)) as i32 - self.display_offset() as i32;
        let column = point.column.min(self.columns().saturating_sub(1));
        let side = if point.right_half {
            Side::Right
        } else {
            Side::Left
        };
        (Point::new(Line(line), Column(column)), side)
    }

    /// Commence une sélection au point donné.
    pub fn start_selection(&mut self, point: ScreenPoint, kind: SelectionKind) {
        let (point, side) = self.grid_point(point);
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, point, side));
    }

    /// Étend la sélection en cours jusqu'au point donné.
    pub fn extend_selection(&mut self, point: ScreenPoint) {
        let (point, side) = self.grid_point(point);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// Vide l'historique (l'écran affiché reste : conhost en garde la copie et
    /// n'envoie que des différences, l'effacer ici le désynchroniserait).
    pub fn clear_history(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
        self.term.grid_mut().clear_history();
        self.term.selection = None;
    }

    /// Texte sélectionné (`None` sans sélection ou sélection vide).
    pub fn selection_text(&self) -> Option<String> {
        let selection = self.term.selection.as_ref()?;
        if selection.is_empty() {
            return None;
        }
        self.term.selection_to_string().filter(|t| !t.is_empty())
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// Copie la partie affichée dans `screen` (réutilise ses tampons).
    pub fn snapshot(&self, screen: &mut Screen) {
        let grid = self.term.grid();
        let columns = grid.columns();
        let lines = grid.screen_lines();
        let offset = grid.display_offset();
        let colors = self.term.colors();
        let selection = self
            .term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term));

        screen.columns = columns;
        screen.lines = lines;
        screen.display_offset = offset;
        screen.history_size = grid.history_size();
        screen.modes = self.modes();
        screen.has_selection = self.has_selection();
        screen.cells.clear();
        screen.cells.reserve(columns * lines);

        let resolve = |color: AnsiColor, flags: &mut CellFlags| -> Color {
            match color {
                AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
                AnsiColor::Indexed(index) => match colors[index as usize] {
                    Some(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
                    None => Color::Indexed(index),
                },
                AnsiColor::Named(named) => {
                    let index = named as usize;
                    if let Some(rgb) = colors[index] {
                        return Color::Rgb(rgb.r, rgb.g, rgb.b);
                    }
                    match named {
                        NamedColor::Foreground
                        | NamedColor::BrightForeground
                        | NamedColor::Cursor => Color::Foreground,
                        NamedColor::DimForeground => {
                            flags.insert(CellFlags::DIM);
                            Color::Foreground
                        }
                        NamedColor::Background => Color::Background,
                        _ if index < 16 => Color::Indexed(index as u8),
                        // Couleurs « atténuées » : la couleur de base, attribut DIM.
                        _ => {
                            flags.insert(CellFlags::DIM);
                            Color::Indexed((index - NamedColor::DimBlack as usize) as u8)
                        }
                    }
                }
            }
        };

        for view_line in 0..lines {
            let line = Line(view_line as i32 - offset as i32);
            let row = &grid[line];
            for column in 0..columns {
                let cell = &row[Column(column)];
                let mut flags = CellFlags::empty();
                let f = cell.flags;
                if f.contains(Flags::BOLD) {
                    flags.insert(CellFlags::BOLD);
                }
                if f.contains(Flags::ITALIC) {
                    flags.insert(CellFlags::ITALIC);
                }
                if f.intersects(Flags::ALL_UNDERLINES) {
                    flags.insert(CellFlags::UNDERLINE);
                }
                if f.contains(Flags::INVERSE) {
                    flags.insert(CellFlags::INVERSE);
                }
                if f.contains(Flags::DIM) {
                    flags.insert(CellFlags::DIM);
                }
                if f.contains(Flags::STRIKEOUT) {
                    flags.insert(CellFlags::STRIKEOUT);
                }
                if f.contains(Flags::HIDDEN) {
                    flags.insert(CellFlags::HIDDEN);
                }
                if f.contains(Flags::WIDE_CHAR) {
                    flags.insert(CellFlags::WIDE);
                }
                if f.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    flags.insert(CellFlags::WIDE_SPACER);
                }
                if selection
                    .as_ref()
                    .is_some_and(|s| s.contains(Point::new(line, Column(column))))
                {
                    flags.insert(CellFlags::SELECTED);
                }
                let fg = resolve(cell.fg, &mut flags);
                let mut ignored = CellFlags::empty();
                let bg = resolve(cell.bg, &mut ignored);
                screen.cells.push(ScreenCell {
                    ch: if f.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                        ' '
                    } else {
                        cell.c
                    },
                    zerowidth: cell.zerowidth().map(Arc::from),
                    fg,
                    bg,
                    flags,
                });
            }
        }

        let cursor = self.term.renderable_content().cursor;
        screen.cursor = (|| {
            let shape = match cursor.shape {
                AnsiCursorShape::Hidden => return None,
                AnsiCursorShape::Block => CursorShape::Block,
                AnsiCursorShape::Underline => CursorShape::Underline,
                AnsiCursorShape::Beam => CursorShape::Beam,
                AnsiCursorShape::HollowBlock => CursorShape::HollowBlock,
            };
            let line = usize::try_from(cursor.point.line.0 + offset as i32).ok()?;
            (line < lines).then_some(ScreenCursor {
                line,
                column: cursor.point.column.0.min(columns.saturating_sub(1)),
                shape,
            })
        })();
    }

    /// Instantané neuf (commodité pour les tests).
    pub fn screen(&self) -> Screen {
        let mut screen = Screen::default();
        self.snapshot(&mut screen);
        screen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emulator(text: &[u8]) -> Emulator {
        let mut emulator = Emulator::new(20, 5, 100);
        emulator.feed(text);
        emulator
    }

    #[test]
    fn plain_text_and_cursor() {
        let emulator = emulator(b"hello\r\nworld");
        let screen = emulator.screen();
        assert_eq!(screen.line_text(0), "hello");
        assert_eq!(screen.line_text(1), "world");
        let cursor = screen.cursor.unwrap();
        assert_eq!((cursor.line, cursor.column), (1, 5));
    }

    #[test]
    fn utf8_accents_and_wide_chars() {
        let emulator = emulator("é 漢x".as_bytes());
        let screen = emulator.screen();
        assert_eq!(screen.line_text(0), "é 漢x");
        let row = screen.row(0);
        assert!(row[2].flags.contains(CellFlags::WIDE));
        assert!(row[3].flags.contains(CellFlags::WIDE_SPACER));
        assert_eq!(row[4].ch, 'x');
    }

    #[test]
    fn colors_16_256_truecolor_and_attributes() {
        let emulator =
            emulator(b"\x1b[31mR\x1b[0m\x1b[38;5;208mO\x1b[38;2;1;2;3mT\x1b[1;4;7mB\x1b[0mN");
        let row = emulator.screen().row(0).to_vec();
        assert_eq!(row[0].fg, Color::Indexed(1));
        assert_eq!(row[1].fg, Color::Indexed(208));
        assert_eq!(row[2].fg, Color::Rgb(1, 2, 3));
        assert!(row[3].flags.contains(CellFlags::BOLD));
        assert!(row[3].flags.contains(CellFlags::UNDERLINE));
        assert!(row[3].flags.contains(CellFlags::INVERSE));
        assert_eq!(row[4].fg, Color::Foreground);
        assert!(row[4].flags.is_empty());
        assert_eq!(row[4].bg, Color::Background);
    }

    #[test]
    fn bright_colors_and_background() {
        let emulator = emulator(b"\x1b[92;44mX");
        let screen = emulator.screen();
        let cell = &screen.row(0)[0];
        assert_eq!(cell.fg, Color::Indexed(10));
        assert_eq!(cell.bg, Color::Indexed(4));
    }

    #[test]
    fn alternate_screen_restores_primary() {
        let mut emulator = emulator(b"prompt$ ");
        emulator.feed(b"\x1b[?1049h\x1b[H\x1b[2Jvim screen");
        let screen = emulator.screen();
        assert!(screen.modes.alt_screen);
        assert_eq!(screen.line_text(0), "vim screen");
        emulator.feed(b"\x1b[?1049l");
        let screen = emulator.screen();
        assert!(!screen.modes.alt_screen);
        assert_eq!(screen.line_text(0), "prompt$");
    }

    #[test]
    fn scrollback_is_bounded_and_scrollable() {
        let mut emulator = Emulator::new(10, 3, 50);
        for i in 0..200 {
            emulator.feed(format!("{i}\r\n").as_bytes());
        }
        assert_eq!(emulator.history_size(), 50);
        emulator.scroll(ScrollAction::Top);
        assert_eq!(emulator.display_offset(), 50);
        // 200 lignes écrites + une ligne vide : la première visible au sommet
        // de l'historique est la 148.
        assert_eq!(emulator.screen().line_text(0), "148");
        assert!(emulator.screen().cursor.is_none());
        emulator.scroll(ScrollAction::Bottom);
        assert_eq!(emulator.display_offset(), 0);
        assert_eq!(emulator.screen().line_text(1), "199");
        emulator.scroll(ScrollAction::Lines(3));
        emulator.clear_history();
        assert_eq!(emulator.history_size(), 0);
        assert_eq!(emulator.display_offset(), 0);
        assert_eq!(emulator.screen().line_text(1), "199");
    }

    #[test]
    fn view_stays_put_when_scrolled_up_and_output_arrives() {
        let mut emulator = Emulator::new(10, 3, 100);
        for i in 0..20 {
            emulator.feed(format!("{i}\r\n").as_bytes());
        }
        emulator.scroll(ScrollAction::Lines(5));
        let before = emulator.screen().line_text(0);
        emulator.feed(b"new\r\n");
        assert_eq!(emulator.screen().line_text(0), before);
        emulator.scroll(ScrollAction::Bottom);
        emulator.feed(b"more\r\n");
        assert_eq!(emulator.display_offset(), 0);
    }

    #[test]
    fn title_and_bell_signals() {
        let mut emulator = Emulator::new(10, 3, 0);
        let signals = emulator.feed(b"\x1b]0;mon titre\x07\x07");
        assert_eq!(
            signals,
            vec![Signal::Title(Some("mon titre".into())), Signal::Bell]
        );
    }

    #[test]
    fn cursor_position_report_is_answered() {
        let mut emulator = Emulator::new(10, 3, 0);
        let signals = emulator.feed(b"ab\x1b[6n");
        assert_eq!(signals, vec![Signal::Reply(b"\x1b[1;3R".to_vec())]);
    }

    #[test]
    fn modes_are_reported() {
        let mut emulator = Emulator::new(10, 3, 0);
        emulator.feed(b"\x1b[?2004h\x1b[?1h\x1b[?25l");
        let modes = emulator.modes();
        assert!(modes.bracketed_paste);
        assert!(modes.app_cursor);
        assert!(!modes.show_cursor);
        assert!(emulator.screen().cursor.is_none());
    }

    #[test]
    fn resize_changes_grid() {
        let mut emulator = emulator(b"abc");
        emulator.resize(40, 10, 8, 16);
        let screen = emulator.screen();
        assert_eq!((screen.columns, screen.lines), (40, 10));
        assert_eq!(screen.line_text(0), "abc");
        let signals = emulator.feed(b"\x1b[18t");
        assert_eq!(signals, vec![Signal::Reply(b"\x1b[8;10;40t".to_vec())]);
    }

    #[test]
    fn selection_simple_word_and_line() {
        let mut emulator = emulator(b"foo bar baz\r\nsecond");
        let at = |line, column, right_half| ScreenPoint {
            line,
            column,
            right_half,
        };
        emulator.start_selection(at(0, 4, false), SelectionKind::Simple);
        emulator.extend_selection(at(0, 6, true));
        assert_eq!(emulator.selection_text().as_deref(), Some("bar"));
        assert!(emulator.screen().row(0)[5]
            .flags
            .contains(CellFlags::SELECTED));

        emulator.start_selection(at(0, 9, false), SelectionKind::Word);
        assert_eq!(emulator.selection_text().as_deref(), Some("baz"));

        emulator.start_selection(at(1, 2, false), SelectionKind::Line);
        assert_eq!(emulator.selection_text().as_deref(), Some("second\n"));

        emulator.clear_selection();
        assert_eq!(emulator.selection_text(), None);
    }

    #[test]
    fn unknown_sequences_leave_no_garbage() {
        let emulator = emulator(b"a\x1b[?9999h\x1b]777;whatever\x07\x1bP+q544e\x1b\\b");
        assert_eq!(emulator.screen().line_text(0), "ab");
    }

    #[test]
    fn redefined_palette_is_resolved() {
        let emulator = emulator(b"\x1b]4;1;rgb:12/34/56\x07\x1b[31mX");
        assert_eq!(emulator.screen().row(0)[0].fg, Color::Rgb(0x12, 0x34, 0x56));
    }
}
