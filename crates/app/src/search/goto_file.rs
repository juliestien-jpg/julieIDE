//! « Aller au fichier » : fenêtre flottante, champ unique, liste floue fournie
//! par `julie-index`, caractères appariés surlignés, `fichier:ligne` accepté.

use std::path::PathBuf;

use eframe::egui::{self, vec2, FontId, Key, Modifiers, Sense, TextEdit};
use julie_core::{TaskId, TextPosition};
use julie_index::{FileResults, IndexService};

use super::{clicked_outside, floating, highlighted_job};
use crate::icons::{self, Icon};
use crate::texts as t;
use crate::theme::Palette;

pub enum Outcome {
    Pending,
    Close,
    Open {
        path: PathBuf,
        position: Option<TextPosition>,
    },
}

#[derive(Default)]
pub struct GotoFile {
    pub query: String,
    pub results: Option<FileResults>,
    task: Option<TaskId>,
    sent: Option<String>,
    pub selected: usize,
    scroll_to_selected: bool,
    focus_pending: bool,
}

impl GotoFile {
    pub fn new() -> Self {
        Self {
            focus_pending: true,
            ..Default::default()
        }
    }

    /// Résultats reçus : ignorés s'ils ne viennent pas de la dernière demande.
    pub fn on_results(&mut self, task: Option<TaskId>, results: FileResults) {
        if task.is_none() || task != self.task {
            return;
        }
        if self.results.as_ref().map(|r| &r.query) != Some(&results.query) {
            self.selected = 0;
        }
        self.selected = self.selected.min(results.matches.len().saturating_sub(1));
        self.results = Some(results);
    }

    /// La liste correspond à ce qui est tapé.
    pub fn is_settled(&self) -> bool {
        self.results.as_ref().is_some_and(|r| r.query == self.query)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        index: Option<&IndexService>,
        recent: &[PathBuf],
    ) -> Outcome {
        let Some(index) = index else {
            return Outcome::Close;
        };
        if self.sent.as_deref() != Some(self.query.as_str()) {
            self.sent = Some(self.query.clone());
            self.task = Some(index.find_files(self.query.clone(), recent.to_vec()).id());
        }

        let (up, down, page_up, page_down, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if escape {
            return Outcome::Close;
        }
        let count = self.results.as_ref().map_or(0, |r| r.matches.len());
        let before = self.selected;
        if down && count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if page_down && count > 0 {
            self.selected = (self.selected + 8).min(count - 1);
        }
        if page_up {
            self.selected = self.selected.saturating_sub(8);
        }
        if before != self.selected {
            self.scroll_to_selected = true;
        }

        let mut outcome = Outcome::Pending;
        if enter {
            if let Some(results) = &self.results {
                if let Some(m) = results.matches.get(self.selected) {
                    outcome = Outcome::Open {
                        path: m.path.clone(),
                        position: results.position,
                    };
                }
            }
        }

        let rect = floating(ctx, "julie-goto-file", 640.0, p, |ui| {
            ui.label(egui::RichText::new(t::GOTO_TITLE).small().color(p.text_dim));
            let edit = ui.add(
                TextEdit::singleline(&mut self.query)
                    .hint_text(t::GOTO_HINT)
                    .desired_width(f32::INFINITY)
                    .margin(vec2(8.0, 6.0)),
            );
            if self.focus_pending {
                self.focus_pending = false;
                edit.request_focus();
            }
            ui.add_space(4.0);
            let Some(results) = &self.results else {
                ui.weak(t::GOTO_SEARCHING);
                return;
            };
            if !results.index_complete {
                ui.colored_label(p.warn, t::GOTO_INDEXING);
            }
            if results.matches.is_empty() {
                if results.index_complete {
                    ui.weak(if self.query.trim().is_empty() {
                        t::GOTO_NO_RECENT
                    } else {
                        t::GOTO_NONE
                    });
                }
                return;
            }
            let scroll = std::mem::take(&mut self.scroll_to_selected);
            egui::ScrollArea::vertical()
                .id_salt("goto-results")
                .max_height(340.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, m) in results.matches.iter().enumerate() {
                        let (rect, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                        if i == self.selected {
                            ui.painter().rect_filled(rect, 5.0, p.accent_soft);
                            if scroll {
                                ui.scroll_to_rect(rect, None);
                            }
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, 5.0, p.bg_alt);
                        }
                        icons::paint(
                            ui.painter(),
                            egui::Rect::from_center_size(
                                egui::pos2(rect.min.x + 14.0, rect.center().y),
                                vec2(14.0, 14.0),
                            ),
                            Icon::File,
                            if m.library { p.ignored } else { p.text_dim },
                        );
                        let chars: Vec<char> = m.relative.chars().collect();
                        let name: String = chars[m.name_start.min(chars.len())..].iter().collect();
                        let dir: String = chars[..m.name_start.min(chars.len())].iter().collect();
                        let name_ranges: Vec<(usize, usize)> = m
                            .positions
                            .iter()
                            .filter(|&&c| c >= m.name_start)
                            .map(|&c| (c - m.name_start, c - m.name_start + 1))
                            .collect();
                        let dir_ranges: Vec<(usize, usize)> = m
                            .positions
                            .iter()
                            .filter(|&&c| c < m.name_start)
                            .map(|&c| (c, c + 1))
                            .collect();
                        let font = FontId::proportional(14.0);
                        let name_color = if m.library { p.text_dim } else { p.text };
                        let name_job = highlighted_job(
                            &name,
                            &name_ranges,
                            font.clone(),
                            name_color,
                            p.spark_fg,
                            p.spark_bg,
                        );
                        let name_galley = ui.fonts_mut(|f| f.layout_job(name_job));
                        let x = rect.min.x + 30.0;
                        let width = name_galley.size().x;
                        let painter = ui.painter().with_clip_rect(rect);
                        painter.galley(
                            egui::pos2(x, rect.center().y - name_galley.size().y / 2.0),
                            name_galley,
                            name_color,
                        );
                        if !dir.is_empty() {
                            let dir_job = highlighted_job(
                                dir.trim_end_matches('/'),
                                &dir_ranges,
                                FontId::proportional(12.5),
                                p.text_dim,
                                p.spark_fg,
                                p.spark_bg,
                            );
                            let dir_galley = ui.fonts_mut(|f| f.layout_job(dir_job));
                            painter.galley(
                                egui::pos2(
                                    x + width + 10.0,
                                    rect.center().y - dir_galley.size().y / 2.0,
                                ),
                                dir_galley,
                                p.text_dim,
                            );
                        }
                        if response.clicked() {
                            self.selected = i;
                            outcome = Outcome::Open {
                                path: m.path.clone(),
                                position: results.position,
                            };
                        }
                    }
                });
            if results.total > results.matches.len() {
                ui.weak(t::goto_more(results.total - results.matches.len()));
            }
        });
        if matches!(outcome, Outcome::Pending) && clicked_outside(ctx, rect) {
            return Outcome::Close;
        }
        outcome
    }

    pub fn state(&self) -> serde_json::Value {
        let matches: Vec<String> = self
            .results
            .as_ref()
            .map(|r| r.matches.iter().map(|m| m.relative.clone()).collect())
            .unwrap_or_default();
        serde_json::json!({
            "query": self.query,
            "selected": self.selected,
            "total": self.results.as_ref().map(|r| r.total),
            "index_complete": self.results.as_ref().map(|r| r.index_complete),
            "position": self.results.as_ref().and_then(|r| r.position).map(|p| format!("{}:{}", p.line + 1, p.column + 1)),
            "results": matches,
        })
    }
}
