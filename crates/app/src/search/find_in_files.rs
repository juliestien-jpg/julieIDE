//! Recherche dans les fichiers : champ, options (casse, mots entiers, regex),
//! masque de fichiers, portée ; résultats en flux groupés par fichier ; aperçu du
//! résultat sélectionné. Les résultats d'une demande périmée sont ignorés.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, text::LayoutJob, vec2, Color32, FontId, Key, Modifiers, Sense, TextEdit, TextFormat,
};
use julie_core::{TaskId, TextRange};
use julie_index::{
    FileTextMatches, IndexError, IndexService, Preview, SearchScope, TextQuery, TextSearchSummary,
};
use julie_text::{HighlightSpan, Language};

use super::{clicked_outside, floating, highlighted_job};
use crate::icons::{self, Icon};
use crate::tabs::Editors;
use crate::texts as t;
use crate::theme::{self, Palette};

const DEBOUNCE: Duration = Duration::from_millis(300);
const ROW_HEIGHT: f32 = 22.0;
/// Hauteurs au mieux ; réduites quand la fenêtre est basse (150 % sur 720 px).
const RESULTS_HEIGHT: f32 = 250.0;
const PREVIEW_HEIGHT: f32 = 150.0;
const PREVIEW_CONTEXT: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryKey {
    pattern: String,
    regex: bool,
    case_sensitive: bool,
    whole_word: bool,
    mask: String,
    scope: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    File(usize),
    Line(usize, usize),
}

pub enum Outcome {
    Pending,
    Close,
    Open {
        path: PathBuf,
        line: u32,
        range: Option<TextRange>,
    },
}

pub struct FindInFiles {
    pub pattern: String,
    pub mask: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Portée « dossier » active (sinon tout le projet).
    pub scope_dir: Option<PathBuf>,
    /// Dossier proposé (sélectionné dans l'arbre à l'ouverture).
    pub offered_dir: Option<PathBuf>,
    seen: Option<QueryKey>,
    launched: Option<QueryKey>,
    edited_at: Instant,
    task: Option<TaskId>,
    pub running: bool,
    pub error: Option<String>,
    files: Vec<FileTextMatches>,
    rows: Vec<Row>,
    pub summary: Option<TextSearchSummary>,
    selected: Option<(PathBuf, u32)>,
    preview: Option<Preview>,
    preview_task: Option<TaskId>,
    preview_requested: Option<(PathBuf, u32)>,
    focus_pattern: bool,
    scroll_to_selected: bool,
    last_offset: f32,
}

impl FindInFiles {
    pub fn new() -> Self {
        Self {
            pattern: String::new(),
            mask: String::new(),
            case_sensitive: false,
            whole_word: false,
            regex: false,
            scope_dir: None,
            offered_dir: None,
            seen: None,
            launched: None,
            edited_at: Instant::now(),
            task: None,
            running: false,
            error: None,
            files: Vec::new(),
            rows: Vec::new(),
            summary: None,
            selected: None,
            preview: None,
            preview_task: None,
            preview_requested: None,
            focus_pattern: false,
            scroll_to_selected: false,
            last_offset: 0.0,
        }
    }

    /// Ouverture de la fenêtre : texte sélectionné dans l'éditeur (une ligne) et
    /// dossier de l'arbre proposés. Le champ prend le focus.
    pub fn prepare(&mut self, prefill: Option<String>, dir: Option<PathBuf>) {
        self.focus_pattern = true;
        self.offered_dir = dir;
        if let Some(text) = prefill.filter(|s| !s.is_empty() && !s.contains('\n')) {
            self.pattern = text;
        }
    }

    pub fn on_matches(&mut self, task: Option<TaskId>, batch: Vec<FileTextMatches>) {
        if task.is_none() || task != self.task {
            return;
        }
        self.files.extend(batch);
        self.files.sort_by(|a, b| a.relative.cmp(&b.relative));
        self.rebuild_rows();
    }

    pub fn on_finished(&mut self, task: Option<TaskId>, summary: TextSearchSummary) {
        if task.is_none() || task != self.task {
            return;
        }
        self.running = false;
        self.summary = Some(summary);
    }

    pub fn on_preview(&mut self, task: Option<TaskId>, preview: Preview) {
        if task.is_some() && task == self.preview_task {
            self.preview = Some(preview);
        }
    }

    fn rebuild_rows(&mut self) {
        self.rows.clear();
        for (f, file) in self.files.iter().enumerate() {
            self.rows.push(Row::File(f));
            for l in 0..file.lines.len() {
                self.rows.push(Row::Line(f, l));
            }
        }
        if self.selected.is_none() {
            self.selected = self.rows.iter().find_map(|row| match *row {
                Row::Line(f, l) => Some((self.files[f].path.clone(), self.files[f].lines[l].line)),
                _ => None,
            });
            self.preview = None;
            self.preview_requested = None;
        }
    }

    fn key(&self) -> QueryKey {
        QueryKey {
            pattern: self.pattern.clone(),
            regex: self.regex,
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            mask: self.mask.clone(),
            scope: self.scope_dir.clone(),
        }
    }

    fn text_query(&self) -> TextQuery {
        let mut query = TextQuery::new(self.pattern.clone());
        query.regex = self.regex;
        query.case_sensitive = self.case_sensitive;
        query.whole_word = self.whole_word;
        query.file_mask = self.mask.clone();
        if let Some(dir) = &self.scope_dir {
            query.scope = SearchScope::Directory(dir.clone());
        }
        query
    }

    fn clear_results(&mut self) {
        self.files.clear();
        self.rows.clear();
        self.summary = None;
        self.selected = None;
        self.preview = None;
        self.preview_requested = None;
        self.preview_task = None;
        self.running = false;
    }

    fn launch(&mut self, index: &IndexService, editors: &Editors) {
        let key = self.key();
        self.launched = Some(key);
        self.clear_results();
        self.error = None;
        if self.pattern.chars().count() < 2 {
            index.cancel_search();
            self.task = None;
            // Une expression invalide s'annonce dès le premier caractère.
            if self.regex && !self.pattern.is_empty() {
                if let Err(error) = regex::Regex::new(&self.pattern) {
                    self.error = Some(t::find_invalid_regex(&error.to_string()));
                }
            }
            return;
        }
        match index.search_text(self.text_query(), editors.unsaved_files()) {
            Ok(handle) => {
                self.task = Some(handle.id());
                self.running = true;
            }
            Err(error) => {
                index.cancel_search();
                self.task = None;
                self.error = Some(match error {
                    IndexError::InvalidPattern { message } => t::find_invalid_regex(&message),
                    IndexError::InvalidMask { mask, message } => format!("{mask} : {message}"),
                    other => other.to_string(),
                });
            }
        }
    }

    /// Plus de saisie en attente de lancement.
    pub fn is_settled(&self) -> bool {
        self.seen == self.launched || self.seen.is_none()
    }

    /// Relance la recherche tout de suite (action « rechercher » répétée).
    pub fn refresh_now(&mut self) {
        self.launched = None;
        self.edited_at = Instant::now() - DEBOUNCE;
    }

    fn selected_position(&self) -> Option<usize> {
        let (path, line) = self.selected.as_ref()?;
        self.rows.iter().position(|row| match *row {
            Row::Line(f, l) => self.files[f].path == *path && self.files[f].lines[l].line == *line,
            _ => false,
        })
    }

    fn move_selection(&mut self, delta: isize) {
        let lines: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::Line(..)))
            .map(|(i, _)| i)
            .collect();
        if lines.is_empty() {
            return;
        }
        let current = self
            .selected_position()
            .and_then(|pos| lines.iter().position(|&i| i == pos))
            .unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, lines.len() as isize - 1) as usize;
        if let Row::Line(f, l) = self.rows[lines[next]] {
            self.selected = Some((self.files[f].path.clone(), self.files[f].lines[l].line));
            self.scroll_to_selected = true;
        }
    }

    fn open_selected(&self) -> Option<Outcome> {
        let (path, line) = self.selected.as_ref()?;
        let file = self.files.iter().find(|f| &f.path == path)?;
        let hit = file.lines.iter().find(|l| l.line == *line)?;
        Some(Outcome::Open {
            path: path.clone(),
            line: *line,
            range: hit.matches.first().copied(),
        })
    }

    fn request_preview(&mut self, index: &IndexService, editors: &Editors) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        if self.preview_requested.as_ref() == Some(&selected) {
            return;
        }
        self.preview_requested = Some(selected.clone());
        let query = (!self.pattern.is_empty()).then(|| self.text_query());
        if let Ok(handle) = index.preview(
            selected.0.clone(),
            selected.1,
            PREVIEW_CONTEXT,
            query,
            editors.unsaved_text(&selected.0),
        ) {
            self.preview_task = Some(handle.id());
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        index: Option<&IndexService>,
        editors: &Editors,
    ) -> Outcome {
        let Some(index) = index else {
            return Outcome::Close;
        };

        // Raccourcis de la fenêtre.
        let (up, down, page_up, page_down, enter, escape, alt_c, alt_w, alt_x) =
            ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::PageUp),
                    i.consume_key(Modifiers::NONE, Key::PageDown),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(Modifiers::ALT, Key::C),
                    i.consume_key(Modifiers::ALT, Key::W),
                    i.consume_key(Modifiers::ALT, Key::X),
                )
            });
        if escape {
            return Outcome::Close;
        }
        if alt_c {
            self.case_sensitive = !self.case_sensitive;
        }
        if alt_w {
            self.whole_word = !self.whole_word;
        }
        if alt_x {
            self.regex = !self.regex;
        }
        if down {
            self.move_selection(1);
        }
        if up {
            self.move_selection(-1);
        }
        if page_down {
            self.move_selection(8);
        }
        if page_up {
            self.move_selection(-8);
        }

        // Recherche lancée 300 ms après la dernière modification.
        let key = self.key();
        if self.seen.as_ref() != Some(&key) {
            self.seen = Some(key.clone());
            self.edited_at = Instant::now();
        }
        if self.launched.as_ref() != Some(&key) {
            let waited = self.edited_at.elapsed();
            if waited >= DEBOUNCE {
                self.launch(index, editors);
            } else {
                ctx.request_repaint_after(DEBOUNCE - waited);
            }
        }
        self.request_preview(index, editors);

        let mut outcome = Outcome::Pending;
        if enter {
            if let Some(open) = self.open_selected() {
                outcome = open;
            }
        }

        let screen = ctx.content_rect();
        let width = (screen.width() * 0.8).clamp(560.0, 980.0);
        let dark = p.dark;
        // Place disponible sous les champs (≈ 150 pt) : résultats 62 %, aperçu 38 %.
        let budget = (screen.height() - 48.0 - 190.0).max(120.0);
        let results_height = (budget * 0.62).min(RESULTS_HEIGHT);
        let preview_height = (budget * 0.38).min(PREVIEW_HEIGHT);
        let summary = self.summary_text();
        let selected_row = self.selected_position();
        let scroll = std::mem::take(&mut self.scroll_to_selected);
        let rows_len = self.rows.len();
        let mut clicked: Option<(PathBuf, u32)> = None;
        let mut opened = false;
        let rect = floating(ctx, "julie-find-in-files", width, p, |ui| {
            ui.label(egui::RichText::new(t::FIND_TITLE).small().color(p.text_dim));
            ui.horizontal(|ui| {
                let edit = ui.add(
                    TextEdit::singleline(&mut self.pattern)
                        .hint_text(t::FIND_HINT)
                        .desired_width(width - 150.0)
                        .margin(vec2(8.0, 6.0)),
                );
                if self.focus_pattern {
                    self.focus_pattern = false;
                    edit.request_focus();
                }
                toggle(ui, p, &mut self.case_sensitive, "Aa", t::FIND_CASE);
                toggle(ui, p, &mut self.whole_word, "W", t::FIND_WORD);
                toggle(ui, p, &mut self.regex, ".*", t::FIND_REGEX);
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(t::FIND_MASK).color(p.text_dim));
                ui.add(
                    TextEdit::singleline(&mut self.mask)
                        .hint_text(t::FIND_MASK_HINT)
                        .desired_width(200.0),
                );
                ui.label(egui::RichText::new(t::FIND_SCOPE).color(p.text_dim));
                let project_selected = self.scope_dir.is_none();
                if ui
                    .selectable_label(project_selected, t::FIND_SCOPE_PROJECT)
                    .clicked()
                {
                    self.scope_dir = None;
                }
                if let Some(dir) = self.offered_dir.clone().or_else(|| self.scope_dir.clone()) {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if ui
                        .selectable_label(!project_selected, t::find_scope_dir(&name))
                        .on_hover_text(dir.display().to_string())
                        .clicked()
                    {
                        self.scope_dir = Some(dir);
                    }
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(p.error, error);
            } else {
                ui.label(egui::RichText::new(summary).color(p.text_dim));
            }
            ui.add_space(2.0);

            // Résultats.
            let mut area = egui::ScrollArea::vertical()
                .id_salt("find-results")
                .min_scrolled_height(results_height)
                .max_height(results_height)
                .auto_shrink([false, false]);
            if scroll {
                if let Some(i) = selected_row {
                    let top = i as f32 * ROW_HEIGHT;
                    if top < self.last_offset {
                        area = area.vertical_scroll_offset(top);
                    } else if top + ROW_HEIGHT > self.last_offset + results_height {
                        area = area.vertical_scroll_offset(top + ROW_HEIGHT - results_height);
                    }
                }
            }
            let output = area.show_rows(ui, ROW_HEIGHT, rows_len, |ui, range| {
                for i in range {
                    let hit = draw_row(ui, p, &self.files, self.rows[i], selected_row == Some(i));
                    if let Some(((path, line), double)) = hit {
                        clicked = Some((path, line));
                        opened |= double;
                    }
                }
            });
            self.last_offset = output.state.clamped_offset().y;

            ui.separator();
            draw_preview(
                self.preview.as_ref(),
                self.selected.is_some(),
                preview_height,
                ui,
                p,
                dark,
            );
        });
        if let Some(sel) = clicked {
            self.selected = Some(sel);
            if opened {
                if let Some(open) = self.open_selected() {
                    outcome = open;
                }
            }
        }
        if matches!(outcome, Outcome::Pending) && clicked_outside(ctx, rect) {
            return Outcome::Close;
        }
        outcome
    }

    fn summary_text(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if self.pattern.chars().count() < 2 {
            return t::FIND_MIN_CHARS.to_owned();
        }
        let matches: usize = self.files.iter().map(FileTextMatches::match_count).sum();
        let files = self.files.len();
        let mut text = match &self.summary {
            Some(s) => t::find_summary(s.matches, s.files_with_matches),
            None => t::find_summary(matches, files),
        };
        if self.running {
            text.push_str(" — ");
            text.push_str(t::FIND_RUNNING);
        }
        if let Some(s) = &self.summary {
            if s.limit_reached {
                text.push_str(" — ");
                text.push_str(t::FIND_LIMIT);
            }
            if s.skipped_too_large > 0 {
                text.push_str(" — ");
                text.push_str(&t::find_too_large(s.skipped_too_large));
            }
        }
        text
    }

    /// Les résultats seuls, sans saisie, pour l'onglet « Recherche » de la
    /// fenêtre d'outils du bas. Rend l'occurrence sur laquelle on a double-cliqué.
    pub fn show_results_tab(&mut self, ui: &mut egui::Ui, p: &Palette) -> Option<Outcome> {
        ui.horizontal(|ui| {
            if self.pattern.is_empty() {
                ui.weak(t::FIND_TAB_EMPTY);
            } else {
                ui.label(
                    egui::RichText::new(format!("« {} » — {}", self.pattern, self.summary_text()))
                        .color(p.text_dim),
                );
            }
        });
        let selected_row = self.selected_position();
        let mut result = None;
        egui::ScrollArea::vertical()
            .id_salt("find-results-tab")
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
                for i in range {
                    if let Some(((path, line), double)) =
                        draw_row(ui, p, &self.files, self.rows[i], selected_row == Some(i))
                    {
                        self.selected = Some((path, line));
                        if double {
                            result = self.open_selected();
                        }
                    }
                }
            });
        result
    }

    pub fn state(&self) -> serde_json::Value {
        let mut lines = Vec::new();
        for file in &self.files {
            for l in &file.lines {
                if lines.len() >= 60 {
                    break;
                }
                lines.push(format!(
                    "{}:{}: {}",
                    file.relative,
                    l.line + 1,
                    l.snippet.trim()
                ));
            }
        }
        serde_json::json!({
            "pattern": self.pattern,
            "mask": self.mask,
            "case": self.case_sensitive,
            "word": self.whole_word,
            "regex": self.regex,
            "scope": self.scope_dir.as_ref().map(|d| d.display().to_string()),
            "running": self.running,
            "error": self.error,
            "files": self.files.len(),
            "summary": self.summary.as_ref().map(|s| serde_json::json!({
                "matches": s.matches, "lines": s.lines, "files_with_matches": s.files_with_matches,
                "files_searched": s.files_searched, "skipped_binary": s.skipped_binary,
                "skipped_too_large": s.skipped_too_large, "limit_reached": s.limit_reached,
            })),
            "text": self.summary_text(),
            "selected": self.selected.as_ref().map(|(p, l)| format!("{}:{}", p.display(), l + 1)),
            "preview": self.preview.as_ref().map(|p| p.lines.iter().map(|l| format!("{}: {}", l.line + 1, l.text)).collect::<Vec<_>>()),
            "first_lines": lines,
        })
    }
}

impl Default for FindInFiles {
    fn default() -> Self {
        Self::new()
    }
}

fn toggle(ui: &mut egui::Ui, p: &Palette, value: &mut bool, label: &str, tooltip: &str) {
    let text =
        egui::RichText::new(label)
            .monospace()
            .color(if *value { p.spark_fg } else { p.text_dim });
    let mut button = egui::Button::new(text).min_size(vec2(30.0, 26.0));
    if *value {
        button = button.fill(p.accent_soft);
    }
    if ui.add(button).on_hover_text(tooltip).clicked() {
        *value = !*value;
    }
}

/// Dessine une ligne de résultats ; rend `((fichier, ligne), double-clic)` si cliquée.
fn draw_row(
    ui: &mut egui::Ui,
    p: &Palette,
    files: &[FileTextMatches],
    row: Row,
    selected: bool,
) -> Option<((PathBuf, u32), bool)> {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 3.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.bg_alt);
    }
    let painter = ui.painter().with_clip_rect(rect);
    match row {
        Row::File(f) => {
            let file = &files[f];
            icons::paint(
                &painter,
                egui::Rect::from_center_size(
                    egui::pos2(rect.min.x + 12.0, rect.center().y),
                    vec2(13.0, 13.0),
                ),
                Icon::File,
                p.accent,
            );
            let name_start = file.relative.rfind('/').map_or(0, |i| i + 1);
            let (dir, name) = file.relative.split_at(name_start);
            let rect_name = painter.text(
                egui::pos2(rect.min.x + 26.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                FontId::proportional(13.5),
                p.text,
            );
            painter.text(
                egui::pos2(rect_name.max.x + 8.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{}  ({})", dir.trim_end_matches('/'), file.match_count()),
                FontId::proportional(12.5),
                p.text_dim,
            );
            response.clicked().then(|| {
                (
                    (file.path.clone(), file.lines.first().map_or(0, |l| l.line)),
                    response.double_clicked(),
                )
            })
        }
        Row::Line(f, l) => {
            let file = &files[f];
            let hit = &file.lines[l];
            painter.text(
                egui::pos2(rect.min.x + 64.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                (hit.line + 1).to_string(),
                FontId::monospace(12.5),
                p.text_dim,
            );
            let trimmed = hit.snippet.trim_start();
            let skipped = hit.snippet.chars().count() - trimmed.chars().count();
            let ranges: Vec<(usize, usize)> = hit
                .matches
                .iter()
                .filter(|r| r.start.line == r.end.line)
                .filter_map(|r| {
                    let start = (r.start.column as usize)
                        .checked_sub(hit.snippet_start as usize + skipped)?;
                    let end = (r.end.column as usize)
                        .checked_sub(hit.snippet_start as usize + skipped)?;
                    Some((start, end))
                })
                .collect();
            let job = highlighted_job(
                trimmed,
                &ranges,
                FontId::monospace(13.0),
                p.text,
                p.spark_fg,
                p.spark_bg,
            );
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(
                egui::pos2(rect.min.x + 76.0, rect.center().y - galley.size().y / 2.0),
                galley,
                p.text,
            );
            response
                .clicked()
                .then(|| ((file.path.clone(), hit.line), response.double_clicked()))
        }
    }
}

/// Une ligne de l'aperçu : couleurs de syntaxe, occurrences surlignées.
fn preview_job(
    text: &str,
    spans: &[HighlightSpan],
    matches: &[(usize, usize)],
    theme: &crate::editor::EditorTheme,
    font: FontId,
    p: &Palette,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut run_key: Option<(Color32, bool)> = None;
    let flush = |job: &mut LayoutJob, run: &mut String, key: Option<(Color32, bool)>| {
        if let (false, Some((color, hl))) = (run.is_empty(), key) {
            job.append(
                run,
                0.0,
                TextFormat {
                    font_id: font.clone(),
                    color: if hl { p.spark_fg } else { color },
                    background: if hl { p.spark_bg } else { Color32::TRANSPARENT },
                    ..Default::default()
                },
            );
        }
        run.clear();
    };
    for (column, (byte, ch)) in text.char_indices().enumerate() {
        let color = spans
            .iter()
            .find(|s| (s.start as usize) <= byte && byte < s.end as usize)
            .map_or(theme.text, |s| theme.syntax_color(s.kind));
        let hl = matches.iter().any(|&(a, b)| column >= a && column < b);
        if run_key != Some((color, hl)) {
            flush(&mut job, &mut run, run_key);
            run_key = Some((color, hl));
        }
        if ch == '\t' {
            run.push_str("    ");
        } else {
            run.push(ch);
        }
    }
    flush(&mut job, &mut run, run_key);
    job
}

/// Aperçu du résultat sélectionné : lignes colorées, occurrences surlignées.
fn draw_preview(
    preview: Option<&Preview>,
    has_selection: bool,
    height: f32,
    ui: &mut egui::Ui,
    p: &Palette,
    dark: bool,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, theme::editor_theme(dark).background);
    let Some(preview) = preview else {
        painter.text(
            rect.left_top() + vec2(10.0, 8.0),
            egui::Align2::LEFT_TOP,
            if has_selection {
                "…"
            } else {
                t::FIND_NO_PREVIEW
            },
            FontId::proportional(13.0),
            p.text_dim,
        );
        return;
    };
    let editor_theme = theme::editor_theme(dark);
    let text: String = preview
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let highlights = julie_text::highlight_text(Language::from_path(&preview.path), &text);
    let font = FontId::monospace(13.0);
    let line_height = 18.0;
    let first = preview.lines.first().map_or(0, |l| l.line);
    for (i, line) in preview.lines.iter().enumerate() {
        let y = rect.min.y + 4.0 + i as f32 * line_height;
        if y > rect.max.y {
            break;
        }
        if line.line == preview.line {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, y),
                    vec2(rect.width(), line_height),
                ),
                0.0,
                editor_theme.current_line,
            );
        }
        painter.text(
            egui::pos2(rect.min.x + 48.0, y + 1.0),
            egui::Align2::RIGHT_TOP,
            (line.line + 1).to_string(),
            font.clone(),
            editor_theme.line_number,
        );
        let spans = highlights
            .get((line.line - first) as usize)
            .map_or(&[][..], |v| v.as_slice());
        let matches: Vec<(usize, usize)> = line
            .matches
            .iter()
            .filter(|r| r.start.line == r.end.line)
            .map(|r| (r.start.column as usize, r.end.column as usize))
            .collect();
        let job = preview_job(&line.text, spans, &matches, &editor_theme, font.clone(), p);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(egui::pos2(rect.min.x + 58.0, y), galley, editor_theme.text);
    }
}
