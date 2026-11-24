//! Zone d'éditeurs à onglets : ouvrir, activer, fermer, marque de modification,
//! enregistrer, bandeau « changé sur le disque », saut à une ligne.
//!
//! Le contenu et l'édition appartiennent à `julie-text` et au composant
//! `editor/` ; ce module ne fait que les ranger en onglets.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, vec2, FontId, Rect, Sense, Ui};
use julie_core::{Tasks, TextPosition};
use julie_index::UnsavedFiles;
use julie_text::{DiskState, Document, LoadedFile, SaveOutcome};

use crate::editor::{Decorations, EditorView};
use crate::icons::{self, Icon};
use crate::texts as t;
use crate::theme::{self, Palette};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Modifié sur le disque alors que l'onglet est modifié.
    Changed,
    Deleted,
}

pub struct OpenEditor {
    pub document: Document,
    pub view: EditorView,
    pub notice: Option<Notice>,
    /// « Garder les miennes » : le bandeau ne revient pas avant rechargement ou
    /// enregistrement.
    keep_mine: bool,
    confirm_reload: bool,
}

impl OpenEditor {
    pub fn path(&self) -> Option<&Path> {
        self.document.path()
    }
}

/// Où sauter après l'ouverture : position (base zéro) et fin de sélection éventuelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Goto {
    pub position: TextPosition,
    pub end: Option<TextPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabIntent {
    Activate(usize),
    Close(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerIntent {
    Reload(PathBuf),
}

#[derive(Default)]
pub struct Editors {
    pub items: Vec<OpenEditor>,
    pub active: usize,
    previous: Option<PathBuf>,
    pending: HashMap<PathBuf, Option<Goto>>,
    /// Onglets à fermer dès que leur enregistrement a réussi.
    pub close_after_save: HashSet<PathBuf>,
    reveal_active: bool,
    /// Fichiers ouverts récemment, le plus récent en premier.
    pub recent: Vec<PathBuf>,
}

impl Editors {
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.items.iter().position(|e| e.path() == Some(path))
    }

    pub fn active_editor(&self) -> Option<&OpenEditor> {
        self.items.get(self.active)
    }

    pub fn active_editor_mut(&mut self) -> Option<&mut OpenEditor> {
        self.items.get_mut(self.active)
    }

    fn touch_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(50);
    }

    /// Ouvre (ou active) un fichier. Le chargement se fait en tâche de fond ; le
    /// saut a lieu à l'arrivée du contenu.
    pub fn open(&mut self, tasks: &Tasks, path: PathBuf, goto: Option<Goto>) {
        if let Some(index) = self.index_of(&path) {
            self.activate(index);
            if let Some(goto) = goto {
                self.apply_goto(index, goto);
            }
            return;
        }
        if let Some(slot) = self.pending.get_mut(&path) {
            if goto.is_some() {
                *slot = goto;
            }
            return;
        }
        self.pending.insert(path.clone(), goto);
        julie_text::spawn_load(tasks, path);
    }

    pub fn activate(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }
        if index != self.active {
            self.previous = self
                .items
                .get(self.active)
                .and_then(|e| e.path().map(Path::to_path_buf));
        }
        self.active = index;
        self.reveal_active = true;
        let path = self.items[index].path().map(Path::to_path_buf);
        self.items[index].view.request_focus();
        if let Some(path) = path {
            self.touch_recent(&path);
        }
    }

    pub fn activate_relative(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        if n > 1 {
            let next = (self.active as isize + delta).rem_euclid(n);
            self.activate(next as usize);
        }
    }

    /// Ctrl+Tab : revient à l'onglet précédemment actif.
    pub fn activate_previous(&mut self) {
        if let Some(path) = self.previous.clone() {
            if let Some(index) = self.index_of(&path) {
                self.activate(index);
            }
        }
    }

    pub fn apply_goto(&mut self, index: usize, goto: Goto) {
        let Some(editor) = self.items.get_mut(index) else {
            return;
        };
        let buffer = editor.document.buffer();
        let start = buffer.position_to_offset(goto.position);
        let end = goto.end.map(|e| buffer.position_to_offset(e));
        editor.document.set_cursor(start, false);
        if let Some(end) = end {
            editor.document.set_cursor(end, true);
        }
        editor.view.center_line(goto.position.line);
        editor.view.reveal_cursor();
        editor.view.request_focus();
    }

    /// Contenu arrivé : nouvel onglet, ou rechargement d'un onglet existant.
    pub fn on_loaded(&mut self, loaded: LoadedFile, waker: &julie_text::Waker, dark: bool) {
        let path = loaded.path.clone();
        let goto = self.pending.remove(&path).flatten();
        if let Some(index) = self.index_of(&path) {
            let editor = &mut self.items[index];
            editor.document.reload(loaded);
            editor.notice = None;
            editor.keep_mine = false;
            editor.confirm_reload = false;
        } else {
            let mut document = Document::from_loaded(loaded);
            document.enable_syntax(Some(waker.clone()));
            let mut view = EditorView::new(&path);
            view.theme = Some(theme::editor_theme(dark));
            self.items.push(OpenEditor {
                document,
                view,
                notice: None,
                keep_mine: false,
                confirm_reload: false,
            });
            let index = self.items.len() - 1;
            self.activate(index);
        }
        if let Some(goto) = goto {
            if let Some(index) = self.index_of(&path) {
                self.apply_goto(index, goto);
            }
        }
    }

    /// Aucun chargement en cours.
    pub fn is_loaded(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn on_load_failed(&mut self, path: &Path) {
        self.pending.remove(path);
    }

    pub fn set_dark(&mut self, dark: bool) {
        for editor in &mut self.items {
            editor.view.theme = Some(theme::editor_theme(dark));
        }
    }

    pub fn close(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }
        let closed = self.items.remove(index);
        if let Some(path) = closed.path() {
            if self.previous.as_deref() == Some(path) {
                self.previous = None;
            }
        }
        if self.items.is_empty() {
            self.active = 0;
        } else if index < self.active || self.active >= self.items.len() {
            self.active = self.active.saturating_sub(1).min(self.items.len() - 1);
        }
        self.reveal_active = true;
        if let Some(editor) = self.items.get_mut(self.active) {
            editor.view.request_focus();
        }
    }

    pub fn modified_paths(&self) -> Vec<PathBuf> {
        self.items
            .iter()
            .filter(|e| e.document.is_modified())
            .filter_map(|e| e.path().map(Path::to_path_buf))
            .collect()
    }

    pub fn any_modified(&self) -> bool {
        self.items.iter().any(|e| e.document.is_modified())
    }

    /// Enregistre tous les onglets modifiés. Rend les erreurs de préparation
    /// (encodage…) ; les résultats d'écriture arrivent en `TextEvent`.
    pub fn save_all(&mut self, tasks: &Tasks) -> Vec<String> {
        let mut errors = Vec::new();
        for editor in &mut self.items {
            if editor.document.is_modified() {
                match editor.document.save_request() {
                    Ok(request) => {
                        julie_text::spawn_save(tasks, request);
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        errors
    }

    pub fn save_paths(&mut self, tasks: &Tasks, paths: &[PathBuf]) -> Vec<String> {
        let mut errors = Vec::new();
        for editor in &mut self.items {
            let wanted = editor.path().is_some_and(|p| paths.iter().any(|q| q == p));
            if wanted && editor.document.is_modified() {
                match editor.document.save_request() {
                    Ok(request) => {
                        julie_text::spawn_save(tasks, request);
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        errors
    }

    pub fn on_saved(&mut self, outcome: &SaveOutcome) {
        if let Some(index) = self.index_of(&outcome.path) {
            let editor = &mut self.items[index];
            editor.document.mark_saved(outcome);
            editor.notice = None;
            editor.keep_mine = false;
        }
        if self.close_after_save.remove(&outcome.path) {
            if let Some(index) = self.index_of(&outcome.path) {
                self.close(index);
            }
        }
    }

    pub fn check_disk(&self, tasks: &Tasks) {
        let files: Vec<_> = self
            .items
            .iter()
            .filter_map(|e| Some((e.path()?.to_path_buf(), e.document.disk_stamp())))
            .collect();
        if !files.is_empty() {
            julie_text::spawn_check_disk(tasks, files);
        }
    }

    pub fn on_disk_checked(&mut self, states: Vec<(PathBuf, DiskState)>, tasks: &Tasks) {
        for (path, state) in states {
            let Some(index) = self.index_of(&path) else {
                continue;
            };
            let editor = &mut self.items[index];
            match state {
                DiskState::Unchanged => {}
                DiskState::Changed if !editor.document.is_modified() => {
                    julie_text::spawn_load(tasks, path);
                }
                DiskState::Changed => {
                    if !editor.keep_mine {
                        editor.notice = Some(Notice::Changed);
                    }
                }
                DiskState::Deleted => {
                    editor.document.mark_deleted_on_disk();
                    editor.notice = Some(Notice::Deleted);
                }
            }
        }
    }

    /// Contenu en mémoire des onglets modifiés : cherché à la place du disque.
    pub fn unsaved_files(&self) -> UnsavedFiles {
        self.items
            .iter()
            .filter(|e| e.document.is_modified())
            .filter_map(|e| {
                Some((
                    e.path()?.to_path_buf(),
                    Arc::<str>::from(e.document.text().as_str()),
                ))
            })
            .collect()
    }

    pub fn unsaved_text(&self, path: &Path) -> Option<Arc<str>> {
        let editor = self.items.get(self.index_of(path)?)?;
        editor
            .document
            .is_modified()
            .then(|| Arc::<str>::from(editor.document.text().as_str()))
    }

    /// Titres d'onglets : le nom, plus le dossier parent si deux onglets ont le
    /// même nom.
    pub fn titles(&self) -> Vec<String> {
        let names: Vec<String> = self
            .items
            .iter()
            .map(|e| {
                e.path()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| t::UNTITLED.to_owned())
            })
            .collect();
        names
            .iter()
            .zip(&self.items)
            .map(|(name, editor)| {
                let duplicate = names.iter().filter(|n| *n == name).count() > 1;
                let parent = editor
                    .path()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned());
                match (duplicate, parent) {
                    (true, Some(parent)) => format!("{name} — {parent}"),
                    _ => name.clone(),
                }
            })
            .collect()
    }

    // ----- Affichage -----------------------------------------------------------

    pub fn show_tab_bar(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        active_fill: egui::Color32,
    ) -> Vec<TabIntent> {
        let titles = self.titles();
        let mut intents = Vec::new();
        let reveal = std::mem::take(&mut self.reveal_active);
        let (active, items) = (self.active, &self.items);
        let strip = ui.available_rect_before_wrap();
        ui.painter().rect_filled(
            Rect::from_min_size(strip.min, vec2(strip.width(), 30.0)),
            0.0,
            p.bg_alt,
        );
        egui::ScrollArea::horizontal()
            .id_salt("editor-tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (i, title) in titles.iter().enumerate() {
                        let editor = &items[i];
                        let is_active = i == active;
                        let modified = editor.document.is_modified();
                        let color = if is_active { p.text } else { p.text_dim };
                        let galley = ui.painter().layout_no_wrap(
                            title.clone(),
                            FontId::proportional(13.5),
                            color,
                        );
                        let width = galley.size().x + 40.0;
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
                        let hovered = response.hovered();
                        let painter = ui.painter();
                        if is_active {
                            painter.rect_filled(rect, 0.0, active_fill);
                            painter.rect_filled(
                                Rect::from_min_size(
                                    rect.left_bottom() - vec2(0.0, 2.0),
                                    vec2(rect.width(), 2.0),
                                ),
                                0.0,
                                p.accent,
                            );
                        } else if hovered {
                            painter.rect_filled(rect, 0.0, p.bg);
                        }
                        painter.galley(
                            egui::pos2(rect.min.x + 12.0, rect.center().y - galley.size().y / 2.0),
                            galley,
                            color,
                        );
                        let close_rect = Rect::from_center_size(
                            egui::pos2(rect.max.x - 14.0, rect.center().y),
                            vec2(16.0, 16.0),
                        );
                        let close = ui.interact(
                            close_rect,
                            egui::Id::unique(("tab-close", i)),
                            Sense::click(),
                        );
                        let painter = ui.painter();
                        if close.hovered() {
                            painter.rect_filled(close_rect, 3.0, p.accent_soft);
                        }
                        if modified && !close.hovered() {
                            icons::paint(painter, close_rect.shrink(2.0), Icon::Dot, p.warn);
                        } else if hovered || is_active || close.hovered() {
                            icons::paint(painter, close_rect.shrink(3.5), Icon::Close, p.text_dim);
                        }
                        if reveal && is_active {
                            ui.scroll_to_rect(rect, None);
                        }
                        if close.clicked() || response.middle_clicked() {
                            intents.push(TabIntent::Close(i));
                        } else if response.clicked() {
                            intents.push(TabIntent::Activate(i));
                        }
                        if let Some(path) = editor.path() {
                            response.on_hover_text(path.display().to_string());
                        }
                    }
                });
            });
        intents
    }

    /// L'éditeur actif, avec son éventuel bandeau. Rend ce que le bandeau demande.
    pub fn show_active(&mut self, ui: &mut Ui, p: &Palette) -> Option<BannerIntent> {
        let editor = self.items.get_mut(self.active)?;
        let mut intent = None;
        if let Some(notice) = editor.notice {
            let text = match notice {
                Notice::Changed => t::BANNER_CHANGED,
                Notice::Deleted => t::BANNER_DELETED,
            };
            egui::Frame::new()
                .fill(p.accent_soft)
                .inner_margin(egui::Margin::symmetric(10, 5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(p.warn, text);
                        if notice == Notice::Changed {
                            let label = if editor.confirm_reload {
                                t::BANNER_RELOAD_CONFIRM
                            } else {
                                t::BANNER_RELOAD
                            };
                            if ui.button(label).clicked() {
                                if editor.confirm_reload {
                                    editor.confirm_reload = false;
                                    if let Some(path) = editor.path() {
                                        intent = Some(BannerIntent::Reload(path.to_path_buf()));
                                    }
                                } else {
                                    editor.confirm_reload = true;
                                }
                            }
                            if ui.button(t::BANNER_KEEP).clicked() {
                                editor.keep_mine = true;
                                editor.notice = None;
                                editor.confirm_reload = false;
                            }
                        } else if ui.button(t::BANNER_DISMISS).clicked() {
                            editor.notice = None;
                        }
                    });
                });
        }
        let decorations = Decorations::default();
        editor.view.show(ui, &mut editor.document, &decorations);
        intent
    }
}
