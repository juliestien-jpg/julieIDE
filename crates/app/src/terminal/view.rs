//! Le composant egui qui affiche une [`Session`] et lui transmet la saisie.

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Color32, CursorIcon, EventFilter, FontId, Id, PointerButton,
    Pos2, Rect, Response, Sense, Stroke, TextFormat, Ui,
};
use julie_terminal::{
    CellFlags, CursorShape, Screen, ScreenCell, ScreenPoint, ScrollAction, SelectionKind, Session,
};

use super::keys::{self, Action, KeyState};
use super::theme::TerminalTheme;

const PADDING: f32 = 4.0;
const SCROLLBAR_WIDTH: f32 = 8.0;
const MULTI_CLICK_SECONDS: f64 = 0.4;

/// Ce que le terminal demande à son hôte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalIntent {
    /// Ctrl+clic sur un `chemin:ligne[:colonne]` : à ouvrir dans l'éditeur.
    /// `path` est tel qu'affiché (relatif au dossier du shell, ou style MSYS
    /// `/c/...`) ; `line` et `column` sont en base 1 comme à l'écran.
    OpenFile {
        path: String,
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// Résultat d'une image.
#[derive(Debug, Clone)]
pub struct TerminalOutput {
    pub response: Response,
    pub intents: Vec<TerminalIntent>,
    /// Taille en cellules donnée à la session.
    pub columns: u16,
    pub lines: u16,
}

/// Vue d'une session : garde l'instantané de l'écran et l'état de la souris ;
/// la session lui est prêtée à chaque image.
pub struct TerminalView {
    id: Id,
    /// Thème imposé ; à défaut, palette claire ou sombre selon egui.
    pub theme: Option<TerminalTheme>,
    pub font_size: f32,
    screen: Screen,
    generation: u64,
    keys: KeyState,
    last_click: Option<(f64, Pos2, u8)>,
    dragging: bool,
    wheel_remainder: f32,
    focus_requested: bool,
}

impl TerminalView {
    /// `id_salt` distingue les vues entre elles (l'identifiant de la session).
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Id::unique(("julie-terminal", id_salt)),
            theme: None,
            font_size: 13.0,
            screen: Screen::default(),
            generation: 0,
            keys: KeyState::default(),
            last_click: None,
            dragging: false,
            wheel_remainder: 0.0,
            focus_requested: false,
        }
    }

    pub fn id(&self) -> Id {
        self.id
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.id))
    }

    /// Le dernier écran copié de la session.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Affiche le terminal sur toute la place disponible.
    pub fn show(&mut self, ui: &mut Ui, session: &Session) -> TerminalOutput {
        let rect = ui.available_rect_before_wrap();
        self.show_in_rect(ui, rect, session)
    }

    pub fn show_in_rect(&mut self, ui: &mut Ui, rect: Rect, session: &Session) -> TerminalOutput {
        ui.allocate_rect(rect, Sense::hover());
        let response = ui.interact(rect, self.id, Sense::click_and_drag());
        let theme = self
            .theme
            .clone()
            .unwrap_or_else(|| TerminalTheme::for_visuals(ui.visuals()));

        // ----- Géométrie : la place disponible fixe la taille de la session ----
        let font = FontId::monospace(self.font_size);
        let (row_height, cell_width) =
            ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'M')));
        let cell_height = row_height.ceil();
        let text_rect = Rect::from_min_max(
            rect.min + vec2(PADDING, PADDING),
            pos2(rect.right() - SCROLLBAR_WIDTH, rect.bottom() - PADDING),
        );
        let columns = ((text_rect.width() / cell_width).floor() as u16).max(2);
        let lines = ((text_rect.height() / cell_height).floor() as u16).max(1);
        let ppp = ui.ctx().pixels_per_point();
        session.resize(
            columns,
            lines,
            (cell_width * ppp).round() as u16,
            (cell_height * ppp).round() as u16,
        );
        let metrics = Metrics {
            origin: text_rect.min,
            cell_width,
            cell_height,
            row_height,
            font,
        };

        // ----- Focus -------------------------------------------------------
        if self.focus_requested {
            self.focus_requested = false;
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        let focused = ui.memory(|m| m.has_focus(self.id));
        if focused {
            // Tab, flèches et Échap vont au shell, pas à la navigation d'egui.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            self.handle_keyboard(ui, session);
        }

        // ----- Souris ------------------------------------------------------
        let mut intents = Vec::new();
        self.handle_pointer(ui, &response, session, &metrics, &mut intents);
        self.context_menu(&response, session);

        // ----- Molette -----------------------------------------------------
        if ui.rect_contains_pointer(rect) {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0.0 {
                self.wheel_remainder += delta / cell_height;
                let whole = self.wheel_remainder.trunc();
                if whole != 0.0 {
                    self.wheel_remainder -= whole;
                    session.wheel(whole as i32);
                }
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
        }

        // ----- Recopie de l'écran, seulement s'il a bougé ------------------
        if session.generation() != self.generation {
            self.generation = session.snapshot(&mut self.screen);
        }

        // ----- Dessin ------------------------------------------------------
        self.paint(ui, rect, &metrics, &theme, focused);
        if focused {
            self.ime_output(ui, text_rect, &metrics);
        }

        TerminalOutput {
            response,
            intents,
            columns,
            lines,
        }
    }

    fn handle_keyboard(&mut self, ui: &Ui, session: &Session) {
        let (events, modifiers) = ui.input(|i| {
            (
                i.filtered_events(&EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                }),
                i.modifiers,
            )
        });
        for event in &events {
            let has_selection = session.has_selection();
            for action in keys::translate(event, modifiers, has_selection, &mut self.keys) {
                match action {
                    Action::Text(text) => session.send_text(&text),
                    Action::Key(key, modifiers) => {
                        session.send_key(key, modifiers);
                    }
                    Action::Paste(text) => session.paste(&text),
                    Action::Copy => {
                        if let Some(text) = session.selection_text() {
                            ui.ctx().copy_text(text);
                        }
                    }
                    Action::RequestPaste => {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    }
                    Action::Scroll(action) => session.scroll(action),
                }
            }
        }
    }

    fn point_at(&self, pointer: Pos2, metrics: &Metrics) -> ScreenPoint {
        let x = ((pointer.x - metrics.origin.x) / metrics.cell_width).max(0.0);
        let y = ((pointer.y - metrics.origin.y) / metrics.cell_height).max(0.0);
        let column = (x.floor() as usize).min(self.screen.columns.saturating_sub(1));
        ScreenPoint {
            line: (y.floor() as usize).min(self.screen.lines.saturating_sub(1)),
            column,
            right_half: x - column as f32 > 0.5,
        }
    }

    fn handle_pointer(
        &mut self,
        ui: &Ui,
        response: &Response,
        session: &Session,
        metrics: &Metrics,
        intents: &mut Vec<TerminalIntent>,
    ) {
        let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let (pressed, secondary, down, modifiers, time) = ui.input(|i| {
            (
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.button_pressed(PointerButton::Secondary),
                i.pointer.primary_down(),
                i.modifiers,
                i.time,
            )
        });
        if response.hovered() {
            let over_link =
                modifiers.ctrl && self.link_at(self.point_at(pointer, metrics)).is_some();
            ui.ctx().set_cursor_icon(if over_link {
                CursorIcon::PointingHand
            } else {
                CursorIcon::Text
            });
        }
        if (pressed || secondary) && response.hovered() {
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        if pressed && response.hovered() {
            let point = self.point_at(pointer, metrics);
            if modifiers.ctrl {
                if let Some(intent) = self.link_at(point) {
                    intents.push(intent);
                    return;
                }
            }
            let count = match self.last_click {
                Some((t, p, n)) if time - t < MULTI_CLICK_SECONDS && p.distance(pointer) < 6.0 => {
                    n % 3 + 1
                }
                _ => 1,
            };
            self.last_click = Some((time, pointer, count));
            let kind = match count {
                2 => SelectionKind::Word,
                3 => SelectionKind::Line,
                _ => SelectionKind::Simple,
            };
            if count == 1 && modifiers.shift && session.has_selection() {
                session.extend_selection(point);
            } else {
                session.start_selection(point, kind);
            }
            self.dragging = true;
            return;
        }
        if !down {
            self.dragging = false;
            return;
        }
        if !self.dragging {
            return;
        }
        // Glisser au-delà du bord : l'historique défile.
        let top = metrics.origin.y;
        let bottom = top + self.screen.lines as f32 * metrics.cell_height;
        if pointer.y < top {
            session.scroll(ScrollAction::Lines(1));
            ui.ctx().request_repaint();
        } else if pointer.y > bottom {
            session.scroll(ScrollAction::Lines(-1));
            ui.ctx().request_repaint();
        }
        session.extend_selection(self.point_at(pointer, metrics));
    }

    fn context_menu(&mut self, response: &Response, session: &Session) {
        response.context_menu(|ui| {
            let has_selection = session.has_selection();
            if ui
                .add_enabled(has_selection, egui::Button::new(texts::COPY))
                .clicked()
            {
                if let Some(text) = session.selection_text() {
                    ui.ctx().copy_text(text);
                }
                ui.close();
            }
            if ui.button(texts::PASTE).clicked() {
                ui.memory_mut(|m| m.request_focus(self.id));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ui.close();
            }
            if ui.button(texts::CLEAR).clicked() {
                session.clear_history();
                ui.close();
            }
        });
    }

    /// `chemin:ligne[:colonne]` sous le point, d'après l'écran affiché.
    fn link_at(&self, point: ScreenPoint) -> Option<TerminalIntent> {
        let row = self.screen.row(point.line);
        let chars: Vec<char> = row
            .iter()
            .filter(|c| !c.flags.contains(CellFlags::WIDE_SPACER))
            .map(|c| c.ch)
            .collect();
        // Colonne d'écran → indice de caractère (les larges prennent deux cellules).
        let spacers = row[..point.column.min(row.len())]
            .iter()
            .filter(|c| c.flags.contains(CellFlags::WIDE_SPACER))
            .count();
        link_in_line(&chars, point.column.saturating_sub(spacers))
    }

    fn ime_output(&self, ui: &Ui, text_rect: Rect, metrics: &Metrics) {
        let Some(cursor) = self.screen.cursor else {
            return;
        };
        let cursor_rect = Rect::from_min_size(
            metrics.cell_origin(cursor.line, cursor.column),
            vec2(2.0, metrics.cell_height),
        );
        let to_global = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .unwrap_or_default();
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Terminal,
                rect: to_global * text_rect,
                cursor_rect: to_global * cursor_rect,
                should_interrupt_composition: false,
            });
        });
    }

    fn paint(&self, ui: &Ui, rect: Rect, metrics: &Metrics, theme: &TerminalTheme, focused: bool) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme.background);
        let screen = &self.screen;

        for line in 0..screen.lines {
            let row = screen.row(line);
            // Fonds : par plages de même couleur.
            let mut column = 0;
            while column < row.len() {
                let (_, bg) = cell_colors(&row[column], theme);
                let start = column;
                while column < row.len() && cell_colors(&row[column], theme).1 == bg {
                    column += 1;
                }
                if bg != theme.background {
                    let min = metrics.cell_origin(line, start);
                    painter.rect_filled(
                        Rect::from_min_size(
                            min,
                            vec2(
                                (column - start) as f32 * metrics.cell_width,
                                metrics.cell_height,
                            ),
                        ),
                        0.0,
                        bg,
                    );
                }
            }
            // Texte : par plages de même style ; un caractère non ASCII ou large
            // a sa propre plage, placée à sa colonne (les polices de secours
            // n'ont pas la chasse de la police principale).
            let mut column = 0;
            while column < row.len() {
                let cell = &row[column];
                if cell.flags.contains(CellFlags::WIDE_SPACER) {
                    column += 1;
                    continue;
                }
                let style = TextStyle::of(cell, theme);
                let start = column;
                let mut text = String::new();
                if cell.ch.is_ascii() && cell.zerowidth.is_none() {
                    while column < row.len() {
                        let c = &row[column];
                        if !c.ch.is_ascii()
                            || c.zerowidth.is_some()
                            || c.flags.contains(CellFlags::WIDE_SPACER)
                            || TextStyle::of(c, theme) != style
                        {
                            break;
                        }
                        text.push(c.ch);
                        column += 1;
                    }
                } else {
                    text.push(cell.ch);
                    if let Some(extra) = &cell.zerowidth {
                        text.extend(extra.iter());
                    }
                    column += 1;
                }
                let blank = text.chars().all(|c| c == ' ');
                if blank && !style.underline && !style.strikeout {
                    continue;
                }
                self.paint_text(&painter, ui, metrics, line, start, text, &style);
            }
        }

        // Curseur.
        if let Some(cursor) = screen.cursor {
            let cell = &screen.row(cursor.line)[cursor.column];
            let width = if cell.flags.contains(CellFlags::WIDE) {
                2.0
            } else {
                1.0
            } * metrics.cell_width;
            let min = metrics.cell_origin(cursor.line, cursor.column);
            let cell_rect = Rect::from_min_size(min, vec2(width, metrics.cell_height));
            match (focused, cursor.shape) {
                (true, CursorShape::Block) => {
                    painter.rect_filled(cell_rect, 0.0, theme.cursor);
                    if cell.ch != ' ' && !cell.flags.contains(CellFlags::HIDDEN) {
                        let mut style = TextStyle::of(cell, theme);
                        style.color = theme.cursor_text;
                        let mut text = cell.ch.to_string();
                        if let Some(extra) = &cell.zerowidth {
                            text.extend(extra.iter());
                        }
                        self.paint_text(
                            &painter,
                            ui,
                            metrics,
                            cursor.line,
                            cursor.column,
                            text,
                            &style,
                        );
                    }
                }
                (true, CursorShape::Beam) => {
                    painter.rect_filled(
                        Rect::from_min_size(min, vec2(2.0, metrics.cell_height)),
                        0.0,
                        theme.cursor,
                    );
                }
                (true, CursorShape::Underline) => {
                    painter.rect_filled(
                        Rect::from_min_max(
                            pos2(cell_rect.left(), cell_rect.bottom() - 2.0),
                            cell_rect.max,
                        ),
                        0.0,
                        theme.cursor,
                    );
                }
                _ => {
                    painter.rect_stroke(
                        cell_rect.shrink(0.5),
                        0.0,
                        Stroke::new(1.0, theme.cursor),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }

        // Position dans l'historique.
        if screen.history_size > 0 {
            let total = (screen.history_size + screen.lines) as f32;
            let track = Rect::from_min_max(
                pos2(rect.right() - SCROLLBAR_WIDTH + 2.0, rect.top() + 2.0),
                pos2(rect.right() - 2.0, rect.bottom() - 2.0),
            );
            let height = (track.height() * screen.lines as f32 / total).max(16.0);
            // 0 : tout en haut de l'historique ; 1 : en bas (vue courante).
            let fraction = (screen.history_size.saturating_sub(screen.display_offset)) as f32
                / screen.history_size as f32;
            let top = track.top() + (track.height() - height) * fraction.clamp(0.0, 1.0);
            painter.rect_filled(
                Rect::from_min_size(pos2(track.left(), top), vec2(track.width(), height)),
                3.0,
                theme.scrollbar,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_text(
        &self,
        painter: &egui::Painter,
        ui: &Ui,
        metrics: &Metrics,
        line: usize,
        column: usize,
        text: String,
        style: &TextStyle,
    ) {
        let format = TextFormat {
            font_id: metrics.font.clone(),
            color: style.color,
            italics: style.italic,
            underline: if style.underline {
                Stroke::new(1.0, style.color)
            } else {
                Stroke::NONE
            },
            strikethrough: if style.strikeout {
                Stroke::new(1.0, style.color)
            } else {
                Stroke::NONE
            },
            ..Default::default()
        };
        let galley = ui.fonts_mut(|f| f.layout_job(LayoutJob::single_section(text, format)));
        let pos = metrics.cell_origin(line, column)
            + vec2(0.0, (metrics.cell_height - metrics.row_height) / 2.0);
        if style.bold {
            // Pas de graisse monospace par défaut dans egui : double tracé décalé.
            painter.galley(pos + vec2(0.6, 0.0), galley.clone(), style.color);
        }
        painter.galley(pos, galley, style.color);
    }
}

struct Metrics {
    origin: Pos2,
    cell_width: f32,
    cell_height: f32,
    row_height: f32,
    font: FontId,
}

impl Metrics {
    fn cell_origin(&self, line: usize, column: usize) -> Pos2 {
        self.origin
            + vec2(
                column as f32 * self.cell_width,
                line as f32 * self.cell_height,
            )
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TextStyle {
    color: Color32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikeout: bool,
}

impl TextStyle {
    fn of(cell: &ScreenCell, theme: &TerminalTheme) -> Self {
        let flags = cell.flags;
        Self {
            color: cell_colors(cell, theme).0,
            bold: flags.contains(CellFlags::BOLD),
            italic: flags.contains(CellFlags::ITALIC),
            underline: flags.contains(CellFlags::UNDERLINE),
            strikeout: flags.contains(CellFlags::STRIKEOUT),
        }
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// (texte, fond) d'une cellule, attributs appliqués.
fn cell_colors(cell: &ScreenCell, theme: &TerminalTheme) -> (Color32, Color32) {
    let flags = cell.flags;
    let mut fg = theme.color(cell.fg, flags.contains(CellFlags::BOLD));
    let mut bg = theme.color(cell.bg, false);
    if flags.contains(CellFlags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    if flags.contains(CellFlags::DIM) {
        fg = mix(fg, bg, 0.4);
    }
    if flags.contains(CellFlags::HIDDEN) {
        fg = bg;
    }
    if flags.contains(CellFlags::SELECTED) {
        bg = theme.selection;
    }
    (fg, bg)
}

/// Cherche un `chemin:ligne[:colonne]` autour de l'indice `at` d'une ligne.
pub fn link_in_line(chars: &[char], at: usize) -> Option<TerminalIntent> {
    if at >= chars.len() {
        return None;
    }
    let separator = |c: char| c.is_whitespace() || "\"'`()<>[]{}|,;".contains(c);
    if separator(chars[at]) {
        return None;
    }
    let mut start = at;
    while start > 0 && !separator(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && !separator(chars[end]) {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    let token = token.trim_end_matches(['.', ':']);
    // Jusqu'à deux suffixes numériques : `:ligne` puis `:colonne`.
    let mut numbers = Vec::new();
    let mut path = token;
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((head, tail))
                if !tail.is_empty()
                    && tail.chars().all(|c| c.is_ascii_digit())
                    && !head.is_empty() =>
            {
                numbers.push(tail.parse::<u32>().ok()?);
                path = head;
            }
            _ => break,
        }
    }
    numbers.reverse();
    let looks_like_file = path.contains(['/', '\\'])
        || path.rsplit_once('.').is_some_and(|(n, ext)| {
            !n.is_empty() && !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric())
        });
    if numbers.is_empty() || !looks_like_file {
        return None;
    }
    Some(TerminalIntent::OpenFile {
        path: path.to_string(),
        line: numbers.first().copied(),
        column: numbers.get(1).copied(),
    })
}

/// Textes du composant (à reprendre dans le catalogue de `redacteur`).
mod texts {
    pub const COPY: &str = "Copier";
    pub const PASTE: &str = "Coller";
    pub const CLEAR: &str = "Effacer l'historique";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(line: &str, at: usize) -> Option<TerminalIntent> {
        let chars: Vec<char> = line.chars().collect();
        link_in_line(&chars, at)
    }

    fn open(path: &str, line: Option<u32>, column: Option<u32>) -> Option<TerminalIntent> {
        Some(TerminalIntent::OpenFile {
            path: path.into(),
            line,
            column,
        })
    }

    #[test]
    fn finds_path_line_and_column() {
        let text = "PHP Fatal error in src/Foo.php:12 here";
        assert_eq!(link(text, 22), open("src/Foo.php", Some(12), None));
        assert_eq!(
            link(r"at C:\proj\a b.php:3:7", 8),
            None,
            "un espace coupe le chemin"
        );
        assert_eq!(
            link(r"at C:\proj\ab.php:3:7.", 8),
            open(r"C:\proj\ab.php", Some(3), Some(7))
        );
        assert_eq!(
            link("(tests/BarTest.php:45)", 5),
            open("tests/BarTest.php", Some(45), None)
        );
        assert_eq!(
            link("/c/Users/x/app.js:10:2", 3),
            open("/c/Users/x/app.js", Some(10), Some(2))
        );
    }

    #[test]
    fn ignores_non_links() {
        assert_eq!(link("Foo.php", 2), None, "sans numéro de ligne");
        assert_eq!(link("12:30:00", 1), None, "une heure");
        assert_eq!(link("http://x", 6), None);
        assert_eq!(link("a b", 1), None);
    }
}
