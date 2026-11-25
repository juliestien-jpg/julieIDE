//! Fenêtre Usages : un onglet par recherche (dix au plus), arbre « Usages » puis
//! « Appels possibles (type inconnu) » groupé par fichier, extrait de ligne avec
//! l'occurrence surlignée, pied qui dit ce qui n'a pas été cherché.
//!
//! Aucune règle de résolution : les usages et le bilan viennent de `julie-index`.
//! Le mot « tous » n'apparaît jamais ; le compte des appels possibles est toujours dit.

use std::path::PathBuf;

use eframe::egui::{self, vec2, FontId, Id, Key, Modifiers, Sense};
use julie_core::{TaskId, TextRange};
use julie_index::{Declaration, Usage, UsageCertainty, UsagesSummary};

use crate::icons::{self, Icon};
use crate::search::highlighted_job;
use crate::texts as t;
use crate::theme::Palette;

/// Onglets gardés ; le plus ancien est remplacé au-delà.
pub const MAX_TABS: usize = 10;
const ROW_HEIGHT: f32 = 22.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Group(UsageCertainty),
    File(UsageCertainty, usize),
    Usage(usize),
}

pub struct UsagesTab {
    pub id: u64,
    pub symbol: Declaration,
    pub include_libraries: bool,
    pub task: Option<TaskId>,
    pub usages: Vec<Usage>,
    pub summary: Option<UsagesSummary>,
    pub running: bool,
    pub interrupted: bool,
    /// Ctrl+clic sur une déclaration : un seul usage → on y saute au lieu d'ouvrir la fenêtre.
    pub jump_if_single: bool,
    /// La fenêtre a été montrée pour cette recherche.
    pub revealed: bool,
    /// Indice dans `usages`.
    pub selected: Option<usize>,
    rows: Vec<Row>,
    /// Par fichier de chaque groupe : (chemin relatif, indices dans `usages`).
    files: Vec<(UsageCertainty, String, Vec<usize>)>,
    dirty: bool,
    scroll_to_selected: bool,
}

impl UsagesTab {
    fn new(
        id: u64,
        symbol: Declaration,
        include_libraries: bool,
        task: Option<TaskId>,
        jump_if_single: bool,
    ) -> Self {
        Self {
            id,
            symbol,
            include_libraries,
            task,
            usages: Vec::new(),
            summary: None,
            running: true,
            interrupted: false,
            jump_if_single,
            revealed: !jump_if_single,
            selected: None,
            rows: Vec::new(),
            files: Vec::new(),
            dirty: true,
            scroll_to_selected: false,
        }
    }

    pub fn title(&self) -> String {
        short_name(&self.symbol)
    }

    pub fn push(&mut self, batch: Vec<Usage>) {
        self.usages.extend(batch);
        self.dirty = true;
        self.rebuild();
    }

    pub fn finish(&mut self, summary: UsagesSummary) {
        self.summary = Some(summary);
        self.running = false;
        self.task = None;
    }

    pub fn sure_count(&self) -> usize {
        self.usages
            .iter()
            .filter(|u| u.certainty == UsageCertainty::Sure)
            .count()
    }

    pub fn possible_count(&self) -> usize {
        self.usages.len() - self.sure_count()
    }

    fn files_with(&self, certainty: UsageCertainty) -> usize {
        let mut files: Vec<&PathBuf> = self
            .usages
            .iter()
            .filter(|u| u.certainty == certainty)
            .map(|u| &u.path)
            .collect();
        files.sort();
        files.dedup();
        files.len()
    }

    fn rebuild(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.rows.clear();
        self.files.clear();
        let is_member = self.symbol.kind.is_member();
        for certainty in [UsageCertainty::Sure, UsageCertainty::Possible] {
            let mut in_group: Vec<usize> = (0..self.usages.len())
                .filter(|&i| self.usages[i].certainty == certainty)
                .collect();
            if certainty == UsageCertainty::Possible && in_group.is_empty() && !is_member {
                continue;
            }
            in_group.sort_by(|&a, &b| {
                let (a, b) = (&self.usages[a], &self.usages[b]);
                (&a.relative, a.range.start).cmp(&(&b.relative, b.range.start))
            });
            self.rows.push(Row::Group(certainty));
            let mut start = 0;
            while start < in_group.len() {
                let relative = self.usages[in_group[start]].relative.clone();
                let mut end = start;
                while end < in_group.len() && self.usages[in_group[end]].relative == relative {
                    end += 1;
                }
                self.files
                    .push((certainty, relative, in_group[start..end].to_vec()));
                let file_index = self.files.len() - 1;
                self.rows.push(Row::File(certainty, file_index));
                self.rows
                    .extend(in_group[start..end].iter().map(|&i| Row::Usage(i)));
                start = end;
            }
        }
    }

    /// Les usages dans l'ordre d'affichage (sûrs d'abord, par fichier et ligne).
    pub fn ordered(&self) -> Vec<usize> {
        self.rows
            .iter()
            .filter_map(|r| match r {
                Row::Usage(i) => Some(*i),
                _ => None,
            })
            .collect()
    }

    pub fn select(&mut self, usage: Option<usize>) {
        self.selected = usage;
        self.scroll_to_selected = true;
    }

    /// Usage suivant (`delta` = 1) ou précédent (-1), en bouclant.
    pub fn step(&mut self, delta: isize) -> Option<usize> {
        let order = self.ordered();
        if order.is_empty() {
            return None;
        }
        let current = self
            .selected
            .and_then(|s| order.iter().position(|&i| i == s));
        let next = match current {
            None if delta > 0 => 0,
            None => order.len() - 1,
            Some(c) => (c as isize + delta).rem_euclid(order.len() as isize) as usize,
        };
        self.select(Some(order[next]));
        self.selected
    }

    pub fn headline(&self) -> String {
        let (sure, files) = match &self.summary {
            Some(s) => (s.sure, s.files_with_sure),
            None => (self.sure_count(), self.files_with(UsageCertainty::Sure)),
        };
        let possible = match &self.summary {
            Some(s) => s.possible,
            None => self.possible_count(),
        };
        t::usages_headline(
            &self.symbol.qualified_name,
            sure,
            files,
            possible,
            self.running,
        )
    }

    /// Lignes du pied : ce qui a été lu, et surtout ce qui ne l'a pas été.
    pub fn footer(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(s) = &self.summary {
            let mut parts = vec![t::usages_files_read(s.files_searched)];
            if s.library_files_skipped > 0 {
                parts.push(t::usages_libraries_skipped(s.library_files_skipped));
            }
            if s.unreadable > 0 {
                parts.push(t::usages_unreadable(s.unreadable));
            }
            lines.push(parts.join(" ; "));
            if !s.index_complete {
                lines.push(t::USAGES_INDEX_WAS_INCOMPLETE.to_owned());
            }
            if s.dynamic_not_searched {
                lines.push(t::USAGES_HELP_MEMBER.to_owned());
            }
        } else if self.symbol.kind.is_member() {
            lines.push(t::USAGES_HELP_MEMBER.to_owned());
        }
        if self.interrupted && self.summary.is_none() {
            lines.push(t::USAGES_STOPPED.to_owned());
        }
        lines
    }

    pub fn state(&self) -> serde_json::Value {
        let order = self.ordered();
        let rows: Vec<String> = self
            .rows
            .iter()
            .take(80)
            .map(|row| match row {
                Row::Group(UsageCertainty::Sure) => format!("# {}", t::USAGES_GROUP_SURE),
                Row::Group(UsageCertainty::Possible) => {
                    format!("# {}", t::USAGES_GROUP_POSSIBLE)
                }
                Row::File(_, f) => {
                    let (_, relative, list) = &self.files[*f];
                    format!("{relative} ({})", list.len())
                }
                Row::Usage(i) => {
                    let u = &self.usages[*i];
                    format!("  {}: {}", u.range.start.line + 1, u.line_text)
                }
            })
            .collect();
        serde_json::json!({
            "title": self.title(),
            "symbol": self.symbol.qualified_name,
            "headline": self.headline(),
            "running": self.running,
            "interrupted": self.interrupted,
            "include_libraries": self.include_libraries,
            "sure": self.sure_count(),
            "possible": self.possible_count(),
            "usages": order.len(),
            "files_with_sure": self.files_with(UsageCertainty::Sure),
            "selected": self.selected.map(|i| {
                let u = &self.usages[i];
                format!("{}:{}", u.relative, u.range.start.line + 1)
            }),
            "summary": self.summary.as_ref().map(|s| serde_json::json!({
                "sure": s.sure, "possible": s.possible,
                "files_with_sure": s.files_with_sure, "files_with_possible": s.files_with_possible,
                "files_searched": s.files_searched,
                "library_files_skipped": s.library_files_skipped,
                "unreadable": s.unreadable,
                "dynamic_not_searched": s.dynamic_not_searched,
                "index_complete": s.index_complete,
            })),
            "footer": self.footer(),
            "rows": rows,
        })
    }
}

/// « getName », « User », « formatDate » : de quoi nommer l'onglet.
fn short_name(symbol: &Declaration) -> String {
    match symbol.container.rsplit(['\\', '.']).next() {
        Some(container) if symbol.kind.is_member() && !container.is_empty() => {
            format!("{container}::{}", symbol.name)
        }
        _ => symbol.name.clone(),
    }
}

/// Ce que l'utilisatrice a demandé dans la fenêtre à cette image.
#[derive(Debug, PartialEq)]
pub enum UsagesIntent {
    /// Ouvrir l'usage (indice dans `usages`) de l'onglet affiché.
    Open(usize),
    /// Relancer la recherche (case « Inclure les bibliothèques » changée ou bouton).
    Relaunch {
        include_libraries: bool,
    },
    Stop,
    CloseTab(usize),
}

#[derive(Default)]
pub struct UsagesView {
    pub tabs: Vec<UsagesTab>,
    pub active: usize,
    next_id: u64,
}

impl UsagesView {
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn active_tab(&self) -> Option<&UsagesTab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut UsagesTab> {
        self.tabs.get_mut(self.active)
    }

    /// Nouvel onglet pour une recherche lancée. Les recherches en cours des autres
    /// onglets sont arrêtées (le service n'en fait tourner qu'une).
    pub fn start(
        &mut self,
        symbol: Declaration,
        include_libraries: bool,
        task: Option<TaskId>,
        jump_if_single: bool,
    ) {
        for tab in &mut self.tabs {
            if tab.running {
                tab.running = false;
                tab.interrupted = true;
            }
        }
        if self.tabs.len() >= MAX_TABS {
            self.tabs.remove(0);
        }
        self.next_id += 1;
        let mut tab = UsagesTab::new(
            self.next_id,
            symbol,
            include_libraries,
            task,
            jump_if_single,
        );
        tab.rebuild();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
    }

    /// Relance l'onglet actif (même symbole) : ses résultats sont vidés.
    pub fn restart_active(&mut self, include_libraries: bool, task: TaskId) {
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if i != self.active && tab.running {
                tab.running = false;
                tab.interrupted = true;
            }
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.include_libraries = include_libraries;
            tab.usages.clear();
            tab.summary = None;
            tab.running = true;
            tab.interrupted = false;
            tab.task = Some(task);
            tab.selected = None;
            tab.jump_if_single = false;
            tab.revealed = true;
            tab.dirty = true;
            tab.rebuild();
        }
    }

    pub fn close(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            } else if index < self.active {
                self.active -= 1;
            }
        }
    }

    pub fn running(&self) -> bool {
        self.tabs.iter().any(|t| t.running)
    }

    pub fn clear(&mut self) {
        self.tabs.clear();
        self.active = 0;
    }

    /// Dessine la fenêtre : rangée d'onglets, en-tête, arbre, pied.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        indexing: Option<u8>,
    ) -> Vec<UsagesIntent> {
        let mut intents = Vec::new();
        if self.tabs.is_empty() {
            ui.add_space(8.0);
            ui.weak(t::USAGES_EMPTY);
            return intents;
        }
        // Onglets des recherches.
        let mut switch = None;
        // Un seul onglet : le titre est dans l'en-tête, la rangée d'onglets ne prendrait que de la place.
        let several = self.tabs.len() > 1;
        if several {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, tab) in self.tabs.iter().enumerate() {
                    let selected = i == self.active;
                    let title = tab.title();
                    let galley = ui.painter().layout_no_wrap(
                        title.clone(),
                        FontId::proportional(13.0),
                        if selected { p.text } else { p.text_dim },
                    );
                    let (rect, response) =
                        ui.allocate_exact_size(vec2(galley.size().x + 38.0, 22.0), Sense::click());
                    if selected {
                        ui.painter().rect_filled(rect, 4.0, p.accent_soft);
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 4.0, p.bg_alt);
                    }
                    let color = if selected { p.text } else { p.text_dim };
                    ui.painter().galley(
                        egui::pos2(rect.min.x + 10.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        color,
                    );
                    let close_rect = egui::Rect::from_center_size(
                        egui::pos2(rect.max.x - 12.0, rect.center().y),
                        vec2(14.0, 14.0),
                    );
                    let close = ui.interact(
                        close_rect,
                        Id::unique(("usages-tab-close", tab.id)),
                        Sense::click(),
                    );
                    icons::paint(
                        ui.painter(),
                        close_rect.shrink(3.0),
                        Icon::Close,
                        if close.hovered() { p.text } else { p.text_dim },
                    );
                    close.clone().on_hover_text(t::USAGES_CLOSE_TAB);
                    if close.clicked() {
                        intents.push(UsagesIntent::CloseTab(i));
                    } else if response.clicked() {
                        switch = Some(i);
                    }
                    response.on_hover_text(&tab.symbol.qualified_name);
                }
            });
        }
        if let Some(i) = switch {
            self.active = i;
        }
        let active = self.active.min(self.tabs.len().saturating_sub(1));
        let list_id = Id::unique(("usages-list", self.tabs[active].id));
        let tab = &mut self.tabs[active];
        tab.rebuild();

        // En-tête : décompte, case, boutons.
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new(tab.headline())
                    .color(p.text)
                    .strong()
                    .size(13.0),
            );
            let mut include = tab.include_libraries;
            if ui
                .checkbox(&mut include, t::USAGES_INCLUDE_LIBRARIES)
                .on_hover_text(t::USAGES_INCLUDE_TIP)
                .changed()
            {
                intents.push(UsagesIntent::Relaunch {
                    include_libraries: include,
                });
            }
            if tab.running {
                if ui.small_button(t::USAGES_STOP).clicked() {
                    intents.push(UsagesIntent::Stop);
                }
            } else if ui.small_button(t::USAGES_RELAUNCH).clicked() {
                intents.push(UsagesIntent::Relaunch {
                    include_libraries: tab.include_libraries,
                });
            }
            if !several
                && ui
                    .small_button(t::USAGES_CLOSE_TAB)
                    .on_hover_text(t::USAGES_CLOSE_TAB)
                    .clicked()
            {
                intents.push(UsagesIntent::CloseTab(active));
            }
        });
        if let Some(percent) = indexing {
            ui.colored_label(p.warn, t::usages_indexing_banner(percent));
        }

        let footer = tab.footer();
        // Le pied peut passer à la ligne (fenêtre étroite, mise à l'échelle) : on le mesure.
        let footer_height: f32 = footer
            .iter()
            .map(|line| {
                ui.painter()
                    .layout(
                        line.clone(),
                        FontId::proportional(12.0),
                        p.text_dim,
                        ui.available_width(),
                    )
                    .size()
                    .y
                    + ui.spacing().item_spacing.y
            })
            .sum::<f32>()
            + ui.spacing().item_spacing.y;
        let list_height = (ui.available_height() - footer_height).max(60.0);

        // Clavier : la liste doit avoir le focus (clic sur une ligne).
        let focused = ui.memory(|m| m.has_focus(list_id));
        if focused {
            let (up, down, enter) = ui.ctx().input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::Enter)
                        | i.consume_key(Modifiers::NONE, Key::F4),
                )
            });
            if down {
                tab.step(1);
            }
            if up {
                tab.step(-1);
            }
            if enter {
                if let Some(selected) = tab.selected {
                    intents.push(UsagesIntent::Open(selected));
                }
            }
        }

        let list_rect =
            egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), list_height));
        ui.interact(list_rect, list_id, Sense::focusable_noninteractive());
        let scroll_to = std::mem::take(&mut tab.scroll_to_selected);
        let selected_row = tab
            .selected
            .and_then(|s| tab.rows.iter().position(|r| *r == Row::Usage(s)));
        let mut clicked: Option<(usize, bool)> = None;
        let rows_len = tab.rows.len();
        let mut area = egui::ScrollArea::vertical()
            .id_salt(("usages-scroll", tab.id))
            .max_height(list_height)
            .auto_shrink([false, false]);
        if scroll_to {
            if let Some(row) = selected_row {
                area = area.vertical_scroll_offset((row as f32 - 3.0).max(0.0) * ROW_HEIGHT);
            }
        }
        area.show_rows(ui, ROW_HEIGHT, rows_len, |ui, range| {
            for i in range {
                let row = tab.rows[i];
                let selected = selected_row == Some(i);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
                if selected {
                    ui.painter().rect_filled(rect, 3.0, p.accent_soft);
                } else if response.hovered() && matches!(row, Row::Usage(_)) {
                    ui.painter().rect_filled(rect, 3.0, p.bg_alt);
                }
                paint_row(ui, p, rect, row, tab);
                if response.clicked() {
                    ui.memory_mut(|m| m.request_focus(list_id));
                    if let Row::Usage(u) = row {
                        clicked = Some((u, response.double_clicked()));
                    }
                }
            }
        });
        if tab.usages.is_empty() && !tab.running && tab.summary.is_some() {
            ui.weak(t::USAGES_NONE_YET);
        }
        if let Some((usage, double)) = clicked {
            tab.selected = Some(usage);
            if double {
                intents.push(UsagesIntent::Open(usage));
            }
        }
        for line in &footer {
            ui.label(egui::RichText::new(line).size(12.0).color(p.text_dim));
        }
        intents
    }
}

fn paint_row(ui: &egui::Ui, p: &Palette, rect: egui::Rect, row: Row, tab: &UsagesTab) {
    let painter = ui.painter().with_clip_rect(rect);
    let center = rect.center().y;
    match row {
        Row::Group(certainty) => {
            let (label, count, color) = match certainty {
                UsageCertainty::Sure => (t::USAGES_GROUP_SURE, tab.sure_count(), p.text),
                UsageCertainty::Possible => {
                    (t::USAGES_GROUP_POSSIBLE, tab.possible_count(), p.warn)
                }
            };
            icons::paint(
                &painter,
                egui::Rect::from_center_size(
                    egui::pos2(rect.min.x + 10.0, center),
                    vec2(10.0, 10.0),
                ),
                Icon::ChevronDown,
                p.text_dim,
            );
            painter.text(
                egui::pos2(rect.min.x + 24.0, center),
                egui::Align2::LEFT_CENTER,
                format!("{label} ({count})"),
                FontId::proportional(13.5),
                color,
            );
        }
        Row::File(_, f) => {
            let (_, relative, list) = &tab.files[f];
            icons::paint(
                &painter,
                egui::Rect::from_center_size(
                    egui::pos2(rect.min.x + 30.0, center),
                    vec2(13.0, 13.0),
                ),
                Icon::File,
                p.accent,
            );
            let name_start = relative.rfind('/').map_or(0, |i| i + 1);
            let (dir, name) = relative.split_at(name_start);
            let library = tab.usages[list[0]].library;
            let name_rect = painter.text(
                egui::pos2(rect.min.x + 44.0, center),
                egui::Align2::LEFT_CENTER,
                name,
                FontId::proportional(13.5),
                if library { p.text_dim } else { p.text },
            );
            let mut detail = format!("{}  ({})", dir.trim_end_matches('/'), list.len());
            if library {
                detail.push_str(&format!("  {}", t::USAGES_LIBRARY_TAG));
            }
            painter.text(
                egui::pos2(name_rect.max.x + 8.0, center),
                egui::Align2::LEFT_CENTER,
                detail,
                FontId::proportional(12.5),
                p.text_dim,
            );
        }
        Row::Usage(i) => {
            let usage = &tab.usages[i];
            painter.text(
                egui::pos2(rect.min.x + 84.0, center),
                egui::Align2::RIGHT_CENTER,
                (usage.range.start.line + 1).to_string(),
                FontId::monospace(12.5),
                p.text_dim,
            );
            let ranges = highlight_columns(usage);
            let job = highlighted_job(
                &usage.line_text,
                &ranges,
                FontId::monospace(13.0),
                p.text,
                p.spark_fg,
                p.spark_bg,
            );
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let width = galley.size().x;
            painter.galley(
                egui::pos2(rect.min.x + 96.0, center - galley.size().y / 2.0),
                galley,
                p.text,
            );
            if usage.unsaved {
                painter.text(
                    egui::pos2(rect.min.x + 96.0 + width + 12.0, center),
                    egui::Align2::LEFT_CENTER,
                    t::USAGES_UNSAVED_TAG,
                    FontId::proportional(11.5),
                    p.warn,
                );
            }
        }
    }
}

/// Colonnes de l'occurrence dans l'extrait (`line_text` commence à `line_text_start`).
fn highlight_columns(usage: &Usage) -> Vec<(usize, usize)> {
    let range: TextRange = usage.range;
    if range.start.line != range.end.line {
        return Vec::new();
    }
    let start = (range.start.column as usize).checked_sub(usage.line_text_start as usize);
    let end = (range.end.column as usize).checked_sub(usage.line_text_start as usize);
    match (start, end) {
        (Some(s), Some(e)) => vec![(s, e)],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_core::TextPosition;
    use julie_index::{SymbolKind, SymbolLanguage};

    fn symbol(kind: SymbolKind) -> Declaration {
        Declaration {
            kind,
            language: SymbolLanguage::Php,
            name: "getName".into(),
            qualified_name: "App\\Model\\User::getName".into(),
            container: "App\\Model\\User".into(),
            path: PathBuf::from("src/Model/User.php"),
            relative: "src/Model/User.php".into(),
            range: TextRange::default(),
            line_text: String::new(),
            library: false,
        }
    }

    fn usage(certainty: UsageCertainty, relative: &str, line: u32) -> Usage {
        Usage {
            certainty,
            path: PathBuf::from(relative),
            relative: relative.to_owned(),
            range: TextRange::new(TextPosition::new(line, 8), TextPosition::new(line, 15)),
            line_text: "$user->getName()".into(),
            line_text_start: 4,
            library: false,
            unsaved: false,
        }
    }

    fn view_with(kind: SymbolKind, usages: Vec<Usage>) -> UsagesView {
        let mut view = UsagesView::default();
        view.start(symbol(kind), false, None, false);
        view.tabs[0].push(usages);
        view
    }

    #[test]
    fn usages_are_grouped_sure_first_then_by_file_and_line() {
        let view = view_with(
            SymbolKind::Method,
            vec![
                usage(UsageCertainty::Possible, "src/b.php", 3),
                usage(UsageCertainty::Sure, "src/b.php", 9),
                usage(UsageCertainty::Sure, "src/a.php", 5),
                usage(UsageCertainty::Sure, "src/b.php", 2),
            ],
        );
        let tab = &view.tabs[0];
        let order: Vec<(String, u32)> = tab
            .ordered()
            .into_iter()
            .map(|i| {
                (
                    tab.usages[i].relative.clone(),
                    tab.usages[i].range.start.line,
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("src/a.php".to_owned(), 5),
                ("src/b.php".to_owned(), 2),
                ("src/b.php".to_owned(), 9),
                ("src/b.php".to_owned(), 3),
            ]
        );
    }

    #[test]
    fn headline_always_says_the_possible_count_and_never_says_all() {
        let view = view_with(SymbolKind::Method, vec![]);
        let headline = view.tabs[0].headline();
        assert!(headline.contains("0 appel possible"), "{headline}");
        assert!(headline.contains("recherche en cours"), "{headline}");
        assert!(!headline.to_lowercase().contains("tous"));
        let view = view_with(
            SymbolKind::Method,
            vec![
                usage(UsageCertainty::Sure, "a.php", 1),
                usage(UsageCertainty::Sure, "a.php", 2),
                usage(UsageCertainty::Possible, "b.php", 1),
            ],
        );
        let headline = view.tabs[0].headline();
        assert!(
            headline.contains("2 usages sûrs dans 1 fichier"),
            "{headline}"
        );
        assert!(headline.contains("1 appel possible"), "{headline}");
    }

    #[test]
    fn stepping_wraps_around_and_starts_at_the_first_usage() {
        let mut view = view_with(
            SymbolKind::Function,
            vec![
                usage(UsageCertainty::Sure, "a.php", 1),
                usage(UsageCertainty::Sure, "a.php", 7),
            ],
        );
        let tab = &mut view.tabs[0];
        let first = tab.step(1).unwrap();
        assert_eq!(tab.usages[first].range.start.line, 1);
        let second = tab.step(1).unwrap();
        assert_eq!(tab.usages[second].range.start.line, 7);
        assert_eq!(tab.step(1), Some(first));
        assert_eq!(tab.step(-1), Some(second));
    }

    #[test]
    fn only_ten_tabs_are_kept_and_older_searches_are_marked_interrupted() {
        let mut view = UsagesView::default();
        for _ in 0..12 {
            view.start(symbol(SymbolKind::Class), false, None, false);
        }
        assert_eq!(view.tabs.len(), MAX_TABS);
        assert!(view.tabs[..MAX_TABS - 1]
            .iter()
            .all(|t| t.interrupted && !t.running));
        assert!(view.tabs[MAX_TABS - 1].running);
    }

    #[test]
    fn the_member_help_line_is_in_the_footer_for_members_only() {
        let member = view_with(SymbolKind::Method, vec![]);
        assert!(member.tabs[0]
            .footer()
            .iter()
            .any(|l| l.contains("appels dynamiques")));
        let class = view_with(SymbolKind::Class, vec![]);
        assert!(class.tabs[0].footer().is_empty());
    }
}
