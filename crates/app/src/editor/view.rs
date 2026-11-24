//! Le composant egui : affichage des lignes visibles, saisie, souris, défilement.

use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, pos2, text::CCursor, text::LayoutJob, vec2, Align2, Color32, CornerRadius, CursorIcon,
    Event, EventFilter, FontId, Galley, Id, ImeEvent, Key, Modifiers, PointerButton, Pos2, Rect,
    Response, Sense, Stroke, TextFormat, Ui, Vec2,
};
use julie_core::TextPosition;
use julie_text::{Document, Motion, Selection, Selections, TAB_WIDTH};

use super::decorations::{Decorations, MarkShape};
use super::layout::EditorLayout;
use super::theme::EditorTheme;

/// Au-delà, une ligne n'est mise en page que sur la partie visible.
const LONG_LINE: usize = 2_000;
/// Caractères mis en page de part et d'autre de la partie visible d'une longue ligne.
const WINDOW_MARGIN: usize = 200;
const MARKS_WIDTH: f32 = 14.0;
const TEXT_PADDING: f32 = 4.0;
const SCROLLBAR_WIDTH: f32 = 10.0;
const MULTI_CLICK_SECONDS: f64 = 0.4;

/// Réglages d'un éditeur.
#[derive(Debug, Clone, PartialEq)]
pub struct EditorOptions {
    pub font_size: f32,
    /// Aucune modification depuis le clavier ni la souris (copier reste possible).
    pub read_only: bool,
    pub line_numbers: bool,
    pub highlight_current_line: bool,
    pub match_brackets: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            font_size: 14.0,
            read_only: false,
            line_numbers: true,
            highlight_current_line: true,
            match_brackets: true,
        }
    }
}

/// Ce que l'éditeur demande à son hôte de faire : il ne sait pas le faire seul.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorIntent {
    /// Clic sur une marque de gouttière (`GutterMark::id`).
    GutterMarkClicked { line: u32, id: u64 },
}

/// Résultat d'une image.
#[derive(Debug, Clone)]
pub struct EditorOutput {
    pub response: Response,
    /// Le texte a changé à cette image.
    pub changed: bool,
    /// Les sélections ont changé (curseur déplacé, sélection, modification).
    pub selection_changed: bool,
    /// Le défilement vertical a bougé du fait de l'utilisatrice (molette, barre,
    /// curseur amené à l'écran) : les volets liés doivent suivre.
    pub scrolled: bool,
    pub intents: Vec<EditorIntent>,
    pub layout: EditorLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Char,
    Word,
    Line,
}

#[derive(Debug, Clone)]
struct Drag {
    granularity: Granularity,
    /// Plage d'origine (le caractère, le mot ou la ligne cliqué).
    origin: Range<usize>,
}

/// Mise en page d'une ligne visible.
struct LineGeometry {
    galley: Option<Arc<Galley>>,
    /// Colonne du premier caractère mis en page.
    first_col: usize,
    /// Caractères mis en page.
    len: usize,
    line_len: usize,
    /// Abscisse (dans le texte, hors défilement) du premier caractère mis en page.
    x0: f32,
    width: f32,
    /// Indice dans le texte affiché (tabulations développées) de chaque caractère
    /// mis en page, plus la fin ; `None` s'il n'y a pas de tabulation.
    display: Option<Vec<u32>>,
}

impl LineGeometry {
    fn x_of(&self, column: usize, char_width: f32) -> f32 {
        if column < self.first_col {
            return column as f32 * char_width;
        }
        let i = column - self.first_col;
        if i > self.len {
            return self.x0 + self.width + (i - self.len) as f32 * char_width;
        }
        let d = self.display.as_ref().map_or(i as u32, |m| m[i]);
        self.x0
            + self
                .galley
                .as_ref()
                .map_or(0.0, |g| g.pos_from_cursor(CCursor::new(d as usize)).min.x)
    }

    fn column_at(&self, x: f32, char_width: f32) -> usize {
        if x < self.x0 {
            return ((x / char_width).round().max(0.0) as usize).min(self.first_col);
        }
        let Some(galley) = &self.galley else {
            return self.first_col;
        };
        if x > self.x0 + self.width {
            let beyond = ((x - self.x0 - self.width) / char_width).round() as usize;
            return (self.first_col + self.len + beyond).min(self.line_len);
        }
        let d = galley
            .cursor_from_pos(vec2(x - self.x0, galley.size().y / 2.0))
            .index
            .0 as u32;
        let i = match &self.display {
            None => d as usize,
            Some(map) => {
                let p = map.partition_point(|&v| v < d);
                if p < map.len() && map[p] == d || p == 0 {
                    p
                } else if p >= map.len() {
                    map.len() - 1
                } else {
                    // Dans une tabulation développée : le bord le plus proche.
                    let left = map[p - 1];
                    let right = map[p];
                    if d - left <= right - d {
                        p - 1
                    } else {
                        p
                    }
                }
            }
        };
        self.first_col + i.min(self.len)
    }
}

/// L'éditeur d'un document. Garde l'état d'affichage (défilement, glisser,
/// composition IME) ; le document lui est prêté à chaque image.
pub struct EditorView {
    id: Id,
    pub options: EditorOptions,
    /// Thème imposé ; à défaut, palette claire ou sombre selon egui.
    pub theme: Option<EditorTheme>,
    scroll: Vec2,
    widest: f32,
    line_height: f32,
    drag: Option<Drag>,
    last_click: Option<(f64, Pos2, u8)>,
    preedit: Option<String>,
    reveal_cursor: bool,
    focus_requested: bool,
    pending_center_line: Option<u32>,
    external_scroll: bool,
    layout: Option<EditorLayout>,
}

impl EditorView {
    /// `id_salt` distingue les éditeurs entre eux (chemin du fichier, « diff-gauche »…).
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Id::unique(("julie-editor", id_salt)),
            options: EditorOptions::default(),
            theme: None,
            scroll: Vec2::ZERO,
            widest: 0.0,
            line_height: 18.0,
            drag: None,
            last_click: None,
            preedit: None,
            reveal_cursor: false,
            focus_requested: false,
            pending_center_line: None,
            external_scroll: false,
            layout: None,
        }
    }

    pub fn id(&self) -> Id {
        self.id
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.options.read_only = read_only;
        self
    }

    // ----- Défilement (pour les volets synchronisés) -------------------------

    /// Défilement vertical en points.
    pub fn scroll_y(&self) -> f32 {
        self.scroll.y
    }

    pub fn set_scroll_y(&mut self, y: f32) {
        self.scroll.y = y.max(0.0);
        self.external_scroll = true;
    }

    /// Ligne du haut, fractionnaire.
    pub fn top_line(&self) -> f32 {
        self.scroll.y / self.line_height
    }

    /// Impose la ligne du haut (fractionnaire) ; borné à l'image suivante.
    pub fn set_top_line(&mut self, line: f32) {
        self.set_scroll_y(line * self.line_height);
    }

    /// Amène la ligne au milieu de la zone à l'image suivante (saut depuis une
    /// recherche, une trace de test).
    pub fn center_line(&mut self, line: u32) {
        self.pending_center_line = Some(line);
    }

    /// Amène le curseur à l'écran à l'image suivante.
    pub fn reveal_cursor(&mut self) {
        self.reveal_cursor = true;
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.id))
    }

    /// Géométrie de la dernière image.
    pub fn layout(&self) -> Option<&EditorLayout> {
        self.layout.as_ref()
    }

    // ----- Affichage -----------------------------------------------------------

    /// Affiche l'éditeur sur toute la place disponible.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        document: &mut Document,
        decorations: &Decorations,
    ) -> EditorOutput {
        let rect = ui.available_rect_before_wrap();
        self.show_in_rect(ui, rect, document, decorations)
    }

    /// Affiche l'éditeur dans `rect`.
    pub fn show_in_rect(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        document: &mut Document,
        decorations: &Decorations,
    ) -> EditorOutput {
        ui.allocate_rect(rect, Sense::hover());
        let response = ui.interact(rect, self.id, Sense::click_and_drag());
        let theme = self
            .theme
            .clone()
            .unwrap_or_else(|| EditorTheme::for_visuals(ui.visuals()));

        document.poll_syntax();

        let font = FontId::monospace(self.options.font_size);
        let (row_height, char_width) =
            ui.fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'M')));
        let line_height = (row_height + 4.0).round();
        self.line_height = line_height;

        // Géométrie de la zone.
        let line_count = document.buffer().len_lines();
        let digits = line_count.to_string().len().max(3) as f32;
        let numbers_width = if self.options.line_numbers {
            digits * char_width + 2.0 * TEXT_PADDING
        } else {
            0.0
        };
        let gutter_width = numbers_width + MARKS_WIDTH;
        let gutter_rect = Rect::from_min_size(rect.min, vec2(gutter_width, rect.height()));
        let show_hbar =
            self.widest + 2.0 * TEXT_PADDING > rect.width() - gutter_width - SCROLLBAR_WIDTH;
        let text_rect = Rect::from_min_max(
            pos2(rect.left() + gutter_width, rect.top()),
            pos2(
                rect.right() - SCROLLBAR_WIDTH,
                rect.bottom() - if show_hbar { SCROLLBAR_WIDTH } else { 0.0 },
            ),
        );

        let scroll_before = self.scroll;
        let external = std::mem::take(&mut self.external_scroll);
        let mut selection_changed = false;
        let mut changed = false;
        let mut intents = Vec::new();

        if self.focus_requested {
            self.focus_requested = false;
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        let focused = ui.memory(|m| m.has_focus(self.id));
        if focused {
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
        }

        let metrics = Metrics {
            font: font.clone(),
            line_height,
            row_height,
            char_width,
            text_rect,
            gutter_rect,
            numbers_width,
        };

        // ----- Souris ------------------------------------------------------
        let selections_before = document.selections().clone();
        self.handle_pointer(
            ui,
            &response,
            document,
            decorations,
            &metrics,
            &theme,
            &mut intents,
        );

        // ----- Molette -----------------------------------------------------
        if ui.rect_contains_pointer(rect) {
            let (delta, shift) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers.shift));
            if delta != Vec2::ZERO {
                let delta = if shift && delta.x == 0.0 {
                    vec2(delta.y, 0.0)
                } else {
                    delta
                };
                self.scroll -= delta;
                ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }

        // ----- Clavier -----------------------------------------------------
        if focused {
            let page = ((text_rect.height() / line_height) as usize)
                .saturating_sub(1)
                .max(1);
            let (text_changed, moved) = self.handle_keyboard(ui, document, page);
            changed |= text_changed;
            if text_changed || moved {
                self.reveal_cursor = true;
            }
        }
        selection_changed |= *document.selections() != selections_before;

        // ----- Bornes du défilement, curseur à l'écran ----------------------
        let line_count = document.buffer().len_lines();
        let total_height = line_count as f32 * line_height;
        let max_y = if total_height <= text_rect.height() {
            0.0
        } else {
            total_height - text_rect.height() / 2.0
        };
        if let Some(line) = self.pending_center_line.take() {
            self.scroll.y =
                line as f32 * line_height - text_rect.height() / 2.0 + line_height / 2.0;
        }
        if std::mem::take(&mut self.reveal_cursor) {
            self.scroll_to_cursor(ui, document, &metrics, &theme);
        }
        self.scroll.y = self.scroll.y.clamp(0.0, max_y.max(0.0));
        let max_x = (self.widest + 4.0 * char_width - text_rect.width()).max(0.0);
        self.scroll.x = self.scroll.x.clamp(0.0, max_x);

        // ----- Dessin ------------------------------------------------------
        let first =
            ((self.scroll.y / line_height).floor() as usize).min(line_count.saturating_sub(1));
        let last = (((self.scroll.y + text_rect.height()) / line_height).ceil() as usize + 1)
            .min(line_count);
        let visible = first as u32..last as u32;
        self.paint(
            ui,
            document,
            decorations,
            &metrics,
            &theme,
            visible.clone(),
            focused,
        );
        self.paint_scrollbars(ui, rect, &metrics, &theme, total_height, max_y, show_hbar);

        if focused {
            self.ime_output(ui, document, &metrics, &theme);
        }

        let scrolled = !external && (self.scroll.y - scroll_before.y).abs() > f32::EPSILON;
        let layout = EditorLayout {
            rect,
            gutter_rect,
            text_rect,
            line_height,
            char_width,
            scroll_y: self.scroll.y,
            visible_lines: visible,
            line_count: line_count as u32,
        };
        self.layout = Some(layout.clone());
        EditorOutput {
            response,
            changed,
            selection_changed,
            scrolled,
            intents,
            layout,
        }
    }

    // ----- Mise en page --------------------------------------------------------

    fn geometry(
        &self,
        ui: &Ui,
        document: &Document,
        line: usize,
        metrics: &Metrics,
        theme: &EditorTheme,
    ) -> LineGeometry {
        let buffer = document.buffer();
        let line_len = buffer.line_len(line);
        let (first_col, len) = if line_len > LONG_LINE {
            let first_visible = (self.scroll.x / metrics.char_width) as usize;
            let first = first_visible.saturating_sub(WINDOW_MARGIN).min(line_len);
            let count =
                (metrics.text_rect.width() / metrics.char_width) as usize + 2 * WINDOW_MARGIN;
            (first, count.min(line_len - first))
        } else {
            (0, line_len)
        };
        let x0 = first_col as f32 * metrics.char_width;
        if len == 0 {
            return LineGeometry {
                galley: None,
                first_col,
                len,
                line_len,
                x0,
                width: 0.0,
                display: None,
            };
        }
        let (job, display) = build_job(document, line, first_col, len, &metrics.font, theme);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let width = galley.size().x;
        LineGeometry {
            galley: Some(galley),
            first_col,
            len,
            line_len,
            x0,
            width,
            display,
        }
    }

    /// Ligne et position sous un point de l'écran.
    fn offset_at(
        &self,
        ui: &Ui,
        document: &Document,
        pos: Pos2,
        metrics: &Metrics,
        theme: &EditorTheme,
    ) -> usize {
        let buffer = document.buffer();
        let y = pos.y - metrics.text_rect.top() + self.scroll.y;
        let line =
            ((y / metrics.line_height).floor().max(0.0) as usize).min(buffer.len_lines() - 1);
        let x = pos.x - metrics.text_rect.left() - TEXT_PADDING + self.scroll.x;
        let geometry = self.geometry(ui, document, line, metrics, theme);
        buffer.line_start(line) + geometry.column_at(x, metrics.char_width)
    }

    fn scroll_to_cursor(
        &mut self,
        ui: &Ui,
        document: &Document,
        metrics: &Metrics,
        theme: &EditorTheme,
    ) {
        let position = document.cursor_position();
        let line = position.line as usize;
        let top = line as f32 * metrics.line_height;
        let height = metrics.text_rect.height();
        if top < self.scroll.y {
            self.scroll.y = top;
        } else if top + metrics.line_height > self.scroll.y + height {
            self.scroll.y = top + metrics.line_height - height;
        }
        let geometry = self.geometry(ui, document, line, metrics, theme);
        let x = geometry.x_of(position.column as usize, metrics.char_width);
        self.widest = self.widest.max(x);
        let width = metrics.text_rect.width() - 2.0 * TEXT_PADDING;
        let margin = 4.0 * metrics.char_width;
        if x < self.scroll.x + margin {
            self.scroll.x = (x - margin).max(0.0);
        } else if x > self.scroll.x + width - margin {
            self.scroll.x = x - width + margin;
        }
    }

    // ----- Souris ----------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn handle_pointer(
        &mut self,
        ui: &Ui,
        response: &Response,
        document: &mut Document,
        decorations: &Decorations,
        metrics: &Metrics,
        theme: &EditorTheme,
        intents: &mut Vec<EditorIntent>,
    ) {
        let Some(pointer) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        if response.hovered() && metrics.text_rect.contains(pointer) {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        let (pressed, down, modifiers, time) = ui.input(|i| {
            (
                i.pointer.button_pressed(PointerButton::Primary),
                i.pointer.primary_down(),
                i.modifiers,
                i.time,
            )
        });

        if pressed && response.hovered() {
            ui.memory_mut(|m| m.request_focus(self.id));
            let in_gutter = metrics.gutter_rect.contains(pointer);
            let offset = self.offset_at(ui, document, pointer, metrics, theme);
            let line = document.buffer().line_of(offset) as u32;

            if in_gutter {
                let in_marks = pointer.x >= metrics.gutter_rect.left() + metrics.numbers_width;
                if in_marks {
                    if let Some(mark) = decorations.gutter_in(line..line + 1).next() {
                        intents.push(EditorIntent::GutterMarkClicked { line, id: mark.id });
                        return;
                    }
                }
                let range = document.line_range(line as usize);
                document.set_selections(Selections::single(Selection::new(range.start, range.end)));
                self.drag = Some(Drag {
                    granularity: Granularity::Line,
                    origin: range,
                });
                return;
            }

            let count = match self.last_click {
                Some((t, p, n)) if time - t < MULTI_CLICK_SECONDS && p.distance(pointer) < 6.0 => {
                    n % 3 + 1
                }
                _ => 1,
            };
            self.last_click = Some((time, pointer, count));

            if modifiers.alt && count == 1 {
                let mut selections = document.selections().clone();
                selections.add(Selection::cursor(offset));
                document.set_selections(selections);
                self.drag = None;
                return;
            }
            let (granularity, origin) = match count {
                2 => {
                    document.select_word_at(offset);
                    (Granularity::Word, document.selections().primary().range())
                }
                3 => {
                    let range = document.line_range(line as usize);
                    document
                        .set_selections(Selections::single(Selection::new(range.start, range.end)));
                    (Granularity::Line, range)
                }
                _ => {
                    document.set_cursor(offset, modifiers.shift);
                    let anchor = document.selections().primary().anchor;
                    (Granularity::Char, anchor..anchor)
                }
            };
            self.drag = Some(Drag {
                granularity,
                origin,
            });
            return;
        }

        if !down {
            self.drag = None;
            return;
        }
        let Some(drag) = self.drag.clone() else {
            return;
        };

        // Glisser au-delà du bord : défilement automatique.
        let text = metrics.text_rect;
        if pointer.y < text.top() {
            self.scroll.y -= (text.top() - pointer.y).min(metrics.line_height * 3.0);
            ui.ctx().request_repaint();
        } else if pointer.y > text.bottom() {
            self.scroll.y += (pointer.y - text.bottom()).min(metrics.line_height * 3.0);
            ui.ctx().request_repaint();
        }
        let offset = self.offset_at(ui, document, pointer, metrics, theme);
        let selection = match drag.granularity {
            Granularity::Char => Selection::new(drag.origin.start, offset),
            Granularity::Word | Granularity::Line => {
                let current = if drag.granularity == Granularity::Word {
                    julie_text::movement::word_at(document.buffer(), offset)
                } else {
                    document.line_range(document.buffer().line_of(offset))
                };
                if current.start < drag.origin.start {
                    Selection::new(drag.origin.end, current.start)
                } else {
                    Selection::new(drag.origin.start, current.end.max(drag.origin.end))
                }
            }
        };
        if document.selections().primary() != selection || document.selections().len() > 1 {
            document.set_selections(Selections::single(selection));
        }
    }

    // ----- Clavier -----------------------------------------------------------------

    /// Rend (texte modifié, curseur déplacé).
    fn handle_keyboard(&mut self, ui: &Ui, document: &mut Document, page: usize) -> (bool, bool) {
        let events = ui.input(|i| {
            i.filtered_events(&EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            })
        });
        let editable = !self.options.read_only && !document.is_read_only();
        let mut changed = false;
        let mut moved = false;
        for event in events {
            match event {
                Event::Text(text) if editable => {
                    self.preedit = None;
                    changed |= document.insert_text(&text);
                }
                Event::Copy => {
                    if let Some(text) = document.copy_text() {
                        ui.ctx().copy_text(text);
                    }
                }
                Event::Cut => {
                    let text = if editable {
                        let text = document.cut();
                        changed |= text.is_some();
                        text
                    } else {
                        document.copy_text()
                    };
                    if let Some(text) = text {
                        ui.ctx().copy_text(text);
                    }
                }
                Event::Paste(text) if editable => changed |= document.paste(&text),
                Event::Ime(ime) if editable => match ime {
                    ImeEvent::Preedit { text, .. } => {
                        self.preedit = (!text.is_empty()).then_some(text);
                    }
                    ImeEvent::Commit(text) => {
                        self.preedit = None;
                        if !text.is_empty() {
                            changed |= document.insert_text(&text);
                        }
                    }
                    _ => {}
                },
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let (c, m) = self.on_key(document, key, modifiers, page, editable);
                    changed |= c;
                    moved |= m;
                }
                _ => {}
            }
        }
        (changed, moved)
    }

    /// Rend (texte modifié, curseur déplacé).
    fn on_key(
        &mut self,
        document: &mut Document,
        key: Key,
        m: Modifiers,
        page: usize,
        editable: bool,
    ) -> (bool, bool) {
        let ctrl = m.command;
        let shift = m.shift;
        let motion = |document: &mut Document, motion: Motion| {
            document.move_cursor(motion, shift);
            (false, true)
        };
        let edit = |done: bool| (done, done);
        match key {
            Key::ArrowLeft if !m.alt => {
                motion(document, if ctrl { Motion::WordLeft } else { Motion::Left })
            }
            Key::ArrowRight if !m.alt => motion(
                document,
                if ctrl {
                    Motion::WordRight
                } else {
                    Motion::Right
                },
            ),
            Key::ArrowUp if m.alt && shift => edit(editable && document.move_lines(true)),
            Key::ArrowDown if m.alt && shift => edit(editable && document.move_lines(false)),
            Key::ArrowUp if ctrl => {
                self.scroll.y -= self.line_height;
                (false, false)
            }
            Key::ArrowDown if ctrl => {
                self.scroll.y += self.line_height;
                (false, false)
            }
            Key::ArrowUp if !m.alt => motion(document, Motion::Up),
            Key::ArrowDown if !m.alt => motion(document, Motion::Down),
            Key::Home => motion(
                document,
                if ctrl {
                    Motion::DocumentStart
                } else {
                    Motion::LineStart
                },
            ),
            Key::End => motion(
                document,
                if ctrl {
                    Motion::DocumentEnd
                } else {
                    Motion::LineEnd
                },
            ),
            Key::PageUp => {
                self.scroll.y -= page as f32 * self.line_height;
                motion(document, Motion::PageUp(page))
            }
            Key::PageDown => {
                self.scroll.y += page as f32 * self.line_height;
                motion(document, Motion::PageDown(page))
            }
            Key::Escape => {
                let primary = document.selections().primary();
                if document.selections().len() > 1 || !primary.is_empty() {
                    document.set_selections(Selections::single(Selection::cursor(primary.head)));
                }
                (false, false)
            }
            Key::A if ctrl => {
                document.select_all();
                (false, false)
            }
            _ if !editable => (false, false),
            Key::Backspace => edit(if ctrl {
                document.delete_word_back()
            } else {
                document.backspace()
            }),
            Key::Delete if !shift => edit(if ctrl {
                document.delete_word_forward()
            } else {
                document.delete_forward()
            }),
            Key::Enter if !ctrl && !m.alt => edit(document.insert_newline()),
            Key::Tab if !ctrl && !m.alt => edit(if shift {
                document.outdent()
            } else {
                document.indent()
            }),
            Key::Z if ctrl && shift => edit(document.redo()),
            Key::Z if ctrl => edit(document.undo()),
            // Ctrl+Y supprime la ligne, comme PhpStorm (pas « rétablir »).
            Key::Y if ctrl => edit(document.delete_lines()),
            Key::D if ctrl => edit(document.duplicate()),
            Key::Slash if ctrl => edit(document.toggle_line_comment()),
            _ => (false, false),
        }
    }

    // ----- IME -----------------------------------------------------------------------

    fn ime_output(&self, ui: &Ui, document: &Document, metrics: &Metrics, theme: &EditorTheme) {
        let position = document.cursor_position();
        let geometry = self.geometry(ui, document, position.line as usize, metrics, theme);
        let x = metrics.text_rect.left() + TEXT_PADDING - self.scroll.x
            + geometry.x_of(position.column as usize, metrics.char_width);
        let y =
            metrics.text_rect.top() + position.line as f32 * metrics.line_height - self.scroll.y;
        let cursor_rect = Rect::from_min_size(pos2(x, y), vec2(2.0, metrics.line_height));
        let to_global = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .unwrap_or_default();
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: to_global * metrics.text_rect,
                cursor_rect: to_global * cursor_rect,
                should_interrupt_composition: false,
            });
        });
    }

    // ----- Dessin --------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        ui: &Ui,
        document: &Document,
        decorations: &Decorations,
        metrics: &Metrics,
        theme: &EditorTheme,
        visible: Range<u32>,
        focused: bool,
    ) {
        let buffer = document.buffer();
        let text_rect = metrics.text_rect;
        let painter = ui.painter_at(text_rect);
        let gutter = ui.painter_at(metrics.gutter_rect);
        painter.rect_filled(text_rect, CornerRadius::ZERO, theme.background);
        gutter.rect_filled(
            metrics.gutter_rect,
            CornerRadius::ZERO,
            theme.gutter_background,
        );

        let lh = metrics.line_height;
        let scroll = self.scroll;
        let origin_x = text_rect.left() + TEXT_PADDING - scroll.x;
        let line_top = |line: u32| text_rect.top() + line as f32 * lh - scroll.y;
        let full_row = |line: u32| {
            Rect::from_x_y_ranges(text_rect.x_range(), line_top(line)..=line_top(line) + lh)
        };

        let primary = document.selections().primary();
        let cursor_line = buffer.line_of(primary.head) as u32;

        // Fonds de lignes : ligne courante, puis décorations.
        if self.options.highlight_current_line
            && primary.is_empty()
            && visible.contains(&cursor_line)
        {
            painter.rect_filled(
                full_row(cursor_line),
                CornerRadius::ZERO,
                theme.current_line,
            );
        }
        for decoration in decorations.lines_in(visible.clone()) {
            let start = decoration.lines.start.max(visible.start);
            let end = decoration.lines.end.min(visible.end);
            if start < end {
                let rect =
                    Rect::from_x_y_ranges(text_rect.x_range(), line_top(start)..=line_top(end));
                painter.rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    theme.tone_background(decoration.tone),
                );
            }
        }

        // Mise en page des lignes visibles.
        let geometries: Vec<LineGeometry> = (visible.start..visible.end)
            .map(|line| self.geometry(ui, document, line as usize, metrics, theme))
            .collect();
        for g in &geometries {
            let right = if g.first_col + g.len < g.line_len {
                g.line_len as f32 * metrics.char_width
            } else {
                g.x0 + g.width
            };
            self.widest = self.widest.max(right);
        }
        let geometry = |line: u32| &geometries[(line - visible.start) as usize];
        let column_x =
            |line: u32, column: usize| origin_x + geometry(line).x_of(column, metrics.char_width);

        // Fond d'une plage de colonnes sur une ligne ; `eol` prolonge d'un demi
        // caractère pour montrer la fin de ligne sélectionnée.
        let span_rect = |line: u32, start: usize, end: usize, eol: bool| {
            let x0 = column_x(line, start);
            let mut x1 = column_x(line, end);
            if eol {
                x1 += metrics.char_width / 2.0;
            }
            Rect::from_x_y_ranges(x0..=x1.max(x0 + 1.0), line_top(line)..=line_top(line) + lh)
        };
        let paint_range = |start: TextPosition, end: TextPosition, color: Color32| {
            let first = start.line.max(visible.start);
            let last = end.line.min(visible.end.saturating_sub(1));
            for line in first..=last {
                if line < visible.start || line >= visible.end {
                    continue;
                }
                let line_len = buffer.line_len(line as usize);
                let from = if line == start.line {
                    start.column as usize
                } else {
                    0
                };
                let (to, eol) = if line == end.line {
                    (end.column as usize, false)
                } else {
                    (line_len, true)
                };
                if from < to || eol {
                    painter.rect_filled(span_rect(line, from, to, eol), CornerRadius::ZERO, color);
                }
            }
        };

        for decoration in decorations.ranges_in(visible.clone()) {
            paint_range(
                decoration.range.start,
                decoration.range.end,
                theme.tone_background(decoration.tone),
            );
        }
        let selection_color = if focused {
            theme.selection
        } else {
            theme.selection_unfocused
        };
        for selection in document.selections().iter().filter(|s| !s.is_empty()) {
            paint_range(
                buffer.offset_to_position(selection.start()),
                buffer.offset_to_position(selection.end()),
                selection_color,
            );
        }

        // Parenthèses appariées.
        if focused && self.options.match_brackets && primary.is_empty() {
            if let Some((a, b)) = document.matching_bracket(primary.head) {
                for offset in [a, b] {
                    let p = buffer.offset_to_position(offset);
                    if visible.contains(&p.line) {
                        let rect =
                            span_rect(p.line, p.column as usize, p.column as usize + 1, false);
                        painter.rect_filled(rect, CornerRadius::ZERO, theme.bracket_match);
                    }
                }
            }
        }

        // Texte.
        let text_offset = (lh - metrics.row_height) / 2.0;
        for line in visible.clone() {
            let g = geometry(line);
            if let Some(galley) = &g.galley {
                let pos = pos2(origin_x + g.x0, line_top(line) + text_offset);
                painter.galley(pos, galley.clone(), theme.text);
            }
        }

        // Séparateurs.
        for &(line, tone) in &decorations.separators {
            if line >= visible.start && line <= visible.end {
                let y = line_top(line);
                painter.hline(
                    text_rect.x_range(),
                    y,
                    Stroke::new(1.0, theme.tone_strong(tone)),
                );
            }
        }

        // Curseurs (et composition IME en cours).
        if focused {
            for selection in document.selections().iter() {
                let p = buffer.offset_to_position(selection.head);
                if !visible.contains(&p.line) {
                    continue;
                }
                let x = column_x(p.line, p.column as usize);
                let y = line_top(p.line);
                if let (Some(preedit), true) = (&self.preedit, *selection == primary) {
                    let galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(preedit.clone(), metrics.font.clone(), theme.text)
                    });
                    let rect = Rect::from_min_size(pos2(x, y), vec2(galley.size().x, lh));
                    painter.rect_filled(rect, CornerRadius::ZERO, theme.background);
                    painter.galley(pos2(x, y + text_offset), galley, theme.text);
                    painter.hline(
                        rect.x_range(),
                        rect.bottom() - 2.0,
                        Stroke::new(1.0, theme.ime_underline),
                    );
                    continue;
                }
                painter.rect_filled(
                    Rect::from_min_size(pos2(x - 1.0, y + 1.0), vec2(2.0, lh - 2.0)),
                    CornerRadius::ZERO,
                    theme.cursor,
                );
            }
        }

        // Gouttière : numéros et marques.
        let numbers_right = metrics.gutter_rect.left() + metrics.numbers_width - TEXT_PADDING;
        if self.options.line_numbers {
            for line in visible.clone() {
                let color = if line == cursor_line {
                    theme.line_number_current
                } else {
                    theme.line_number
                };
                gutter.text(
                    pos2(numbers_right, line_top(line) + lh / 2.0),
                    Align2::RIGHT_CENTER,
                    (line + 1).to_string(),
                    metrics.font.clone(),
                    color,
                );
            }
        }
        let marks_left = metrics.gutter_rect.left() + metrics.numbers_width;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        for mark in decorations.gutter_in(visible.clone()) {
            let top = line_top(mark.line);
            let color = theme.tone_strong(mark.tone);
            let area = Rect::from_min_size(pos2(marks_left, top), vec2(MARKS_WIDTH, lh));
            match mark.shape {
                MarkShape::Bar => {
                    let bar = Rect::from_min_size(pos2(marks_left + 3.0, top), vec2(4.0, lh));
                    gutter.rect_filled(bar, CornerRadius::ZERO, color);
                }
                MarkShape::Wedge => {
                    let c = pos2(marks_left + 3.0, top);
                    gutter.add(egui::Shape::convex_polygon(
                        vec![c + vec2(0.0, -4.0), c + vec2(6.0, 0.0), c + vec2(0.0, 4.0)],
                        color,
                        Stroke::NONE,
                    ));
                }
                MarkShape::Run => {
                    let c = area.center();
                    gutter.add(egui::Shape::convex_polygon(
                        vec![
                            c + vec2(-3.5, -5.0),
                            c + vec2(5.0, 0.0),
                            c + vec2(-3.5, 5.0),
                        ],
                        color,
                        Stroke::NONE,
                    ));
                }
                MarkShape::Dot => {
                    gutter.circle_filled(area.center(), 3.5, color);
                }
            }
            if let (Some(tooltip), Some(p)) = (&mark.tooltip, pointer) {
                if area.contains(p) {
                    ui.interact(area, self.id.with(("mark", mark.id)), Sense::hover())
                        .on_hover_text(tooltip.as_str());
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_scrollbars(
        &mut self,
        ui: &Ui,
        rect: Rect,
        metrics: &Metrics,
        theme: &EditorTheme,
        total_height: f32,
        max_y: f32,
        show_hbar: bool,
    ) {
        let painter = ui.painter_at(rect);
        let text = metrics.text_rect;
        // Verticale.
        let track = Rect::from_min_max(
            pos2(text.right(), rect.top()),
            pos2(rect.right(), text.bottom()),
        );
        painter.rect_filled(track, CornerRadius::ZERO, theme.background);
        if max_y > 0.0 {
            let content = total_height.max(max_y + text.height());
            let thumb_height = (text.height() / content * track.height()).max(20.0);
            let travel = track.height() - thumb_height;
            let top = track.top() + travel * (self.scroll.y / max_y).clamp(0.0, 1.0);
            let thumb = Rect::from_min_size(
                pos2(track.left() + 2.0, top),
                vec2(track.width() - 4.0, thumb_height),
            );
            let response = ui.interact(track, self.id.with("vscroll"), Sense::click_and_drag());
            if response.dragged() && travel > 0.0 {
                self.scroll.y =
                    (self.scroll.y + response.drag_delta().y * max_y / travel).clamp(0.0, max_y);
            } else if response.clicked() {
                if let Some(p) = response.interact_pointer_pos() {
                    let ratio = ((p.y - track.top() - thumb_height / 2.0) / travel.max(1.0))
                        .clamp(0.0, 1.0);
                    self.scroll.y = ratio * max_y;
                }
            }
            let color = if response.hovered() || response.dragged() {
                theme.scrollbar_hover
            } else {
                theme.scrollbar
            };
            painter.rect_filled(thumb, CornerRadius::same(3), color);
        }
        // Horizontale.
        if show_hbar {
            let track = Rect::from_min_max(
                pos2(text.left(), text.bottom()),
                pos2(text.right(), rect.bottom()),
            );
            painter.rect_filled(track, CornerRadius::ZERO, theme.background);
            let content = self.widest + 4.0 * metrics.char_width;
            let max_x = (content - text.width()).max(1.0);
            let thumb_width = (text.width() / content * track.width()).clamp(20.0, track.width());
            let travel = track.width() - thumb_width;
            let left = track.left() + travel * (self.scroll.x / max_x).clamp(0.0, 1.0);
            let thumb = Rect::from_min_size(
                pos2(left, track.top() + 2.0),
                vec2(thumb_width, track.height() - 4.0),
            );
            let response = ui.interact(track, self.id.with("hscroll"), Sense::drag());
            if response.dragged() && travel > 0.0 {
                self.scroll.x =
                    (self.scroll.x + response.drag_delta().x * max_x / travel).clamp(0.0, max_x);
            }
            let color = if response.hovered() || response.dragged() {
                theme.scrollbar_hover
            } else {
                theme.scrollbar
            };
            painter.rect_filled(thumb, CornerRadius::same(3), color);
        }
    }
}

struct Metrics {
    font: FontId,
    line_height: f32,
    row_height: f32,
    char_width: f32,
    text_rect: Rect,
    gutter_rect: Rect,
    numbers_width: f32,
}

/// Construit la mise en page d'une partie de ligne : couleurs de la coloration,
/// tabulations développées. Rend aussi la correspondance caractère -> indice
/// affiché quand il y a des tabulations.
fn build_job(
    document: &Document,
    line: usize,
    first_col: usize,
    len: usize,
    font: &FontId,
    theme: &EditorTheme,
) -> (LayoutJob, Option<Vec<u32>>) {
    let content = document.buffer().line(line);
    let window = content.slice(first_col..first_col + len);
    let mut byte = content.char_to_byte(first_col);
    let spans = document.highlights(line);
    let mut span = spans.partition_point(|s| (s.end as usize) <= byte);

    let mut job = LayoutJob {
        break_on_newline: false,
        keep_trailing_whitespace: true,
        ..Default::default()
    };
    job.wrap.max_width = f32::INFINITY;
    let format = |color: Color32| TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };

    let mut run = String::new();
    let mut run_color = theme.text;
    let mut display: Option<Vec<u32>> = None;
    let mut shown = 0u32;
    let mut visual = if first_col == 0 { 0 } else { first_col as u32 };
    for (i, c) in window.chars().enumerate() {
        while span < spans.len() && (spans[span].end as usize) <= byte {
            span += 1;
        }
        let color = match spans.get(span) {
            Some(s) if (s.start as usize) <= byte => theme.syntax_color(s.kind),
            _ => theme.text,
        };
        if color != run_color && !run.is_empty() {
            job.append(&run, 0.0, format(run_color));
            run.clear();
        }
        run_color = color;
        if c == '\t' {
            let width = TAB_WIDTH - visual % TAB_WIDTH;
            display
                .get_or_insert_with(|| (0..i as u32).collect())
                .push(shown);
            run.extend(std::iter::repeat_n(' ', width as usize));
            shown += width;
            visual += width;
        } else {
            if let Some(map) = &mut display {
                map.push(shown);
            }
            run.push(c);
            shown += 1;
            visual += 1;
        }
        byte += c.len_utf8();
    }
    if let Some(map) = &mut display {
        map.push(shown);
    }
    if !run.is_empty() {
        job.append(&run, 0.0, format(run_color));
    }
    (job, display)
}
