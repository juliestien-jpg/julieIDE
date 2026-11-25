//! Barre d'état et suivi des tâches de fond.
//!
//! Les tâches viennent du bus (`Event::Task`). Les sessions de terminal durent
//! toute la vie de leur onglet et les contrôles de disque sont instantanés : on
//! ne les compte pas comme un travail en attente.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Align, FontId, Layout, Sense, Ui};
use julie_core::{TaskEvent, TaskId, TaskState};

use crate::icons::{self, Icon};
use crate::texts as t;
use crate::theme::Palette;

/// Une tâche plus courte que cela ne s'affiche pas (évite le clignotement).
const SHOW_AFTER: Duration = Duration::from_millis(400);

/// Tâches qui ne comptent pas comme « travail en cours ».
const IGNORED_KINDS: &[&str] = &[
    "terminal.session",
    "text.check_disk",
    "index.find_files",
    "index.preview",
];

#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub id: TaskId,
    pub kind: &'static str,
    pub started: Instant,
    pub progress: Option<(u64, u64)>,
}

#[derive(Default)]
pub struct TaskTracker {
    running: HashMap<TaskId, TaskInfo>,
}

impl TaskTracker {
    pub fn on_event(&mut self, event: &TaskEvent) {
        match &event.state {
            TaskState::Started => {
                self.running.insert(
                    event.id,
                    TaskInfo {
                        id: event.id,
                        kind: event.kind,
                        started: Instant::now(),
                        progress: None,
                    },
                );
            }
            TaskState::Progress { done, total } => {
                if let Some(info) = self.running.get_mut(&event.id) {
                    info.progress = Some((*done, *total));
                }
            }
            TaskState::Finished(_) => {
                self.running.remove(&event.id);
            }
        }
    }

    fn counted(&self) -> impl Iterator<Item = &TaskInfo> {
        self.running
            .values()
            .filter(|info| !IGNORED_KINDS.contains(&info.kind))
    }

    /// Du travail de fond est en cours (sans les sessions de terminal).
    pub fn busy(&self) -> bool {
        self.counted().next().is_some()
    }

    /// Les tâches à montrer : en cours depuis plus de 400 ms, la plus ancienne d'abord.
    pub fn visible(&self) -> Vec<TaskInfo> {
        let mut tasks: Vec<TaskInfo> = self
            .counted()
            .filter(|info| info.started.elapsed() >= SHOW_AFTER)
            .cloned()
            .collect();
        tasks.sort_by_key(|info| info.started);
        tasks
    }

    /// Délai avant que la prochaine tâche cachée devienne visible.
    pub fn next_reveal(&self) -> Option<Duration> {
        self.counted()
            .filter_map(|info| SHOW_AFTER.checked_sub(info.started.elapsed()))
            .min()
    }

    /// Les tâches en cours dont le genre satisfait `pred` (annulation des opérations réseau).
    pub fn running_of(&self, pred: impl Fn(&str) -> bool) -> Vec<TaskId> {
        self.running
            .iter()
            .filter(|(_, info)| pred(info.kind))
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        let mut kinds: Vec<&'static str> = self.counted().map(|i| i.kind).collect();
        kinds.sort_unstable();
        kinds
    }
}

/// Ce que la barre d'état affiche, rassemblé par l'application.
pub struct StatusModel {
    pub file: Option<String>,
    pub modified: bool,
    /// Ligne et colonne, à partir de 1.
    pub position: Option<(u32, u32)>,
    pub encoding: Option<&'static str>,
    pub line_ending: Option<&'static str>,
    pub branch: Option<String>,
    pub ahead_behind: Option<(u32, u32)>,
    pub tasks: Vec<TaskInfo>,
    pub message: Option<(String, bool)>,
    /// Opération Git en cours et fichiers en conflit (pastille de la barre).
    pub operation: Option<String>,
    /// Ligne de progression d'une opération réseau (« Réception : 45 % »).
    pub progress_text: Option<String>,
    /// Libellé du bouton de la notification (« Restaurer »…).
    pub message_action: Option<String>,
}

/// Ce que l'utilisatrice a fait dans la barre d'état à cette image.
#[derive(Default)]
pub struct StatusOut {
    /// Annuler cette tâche réseau.
    pub cancel: Option<TaskId>,
    /// Le bouton de la notification a été pressé.
    pub message_action: bool,
}

/// Les tâches réseau se laissent annuler (Git et ses descendants sont tués).
pub fn cancellable(kind: &str) -> bool {
    matches!(
        kind,
        "vcs.fetch" | "vcs.push" | "vcs.update" | "vcs.pull" | "vcs.remote_branch_delete"
    )
}

impl StatusModel {
    /// Texte de la barre pour l'état écrit par le mode scénario.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "file": self.file,
            "modified": self.modified,
            "position": self.position.map(|(l, c)| format!("{l}:{c}")),
            "encoding": self.encoding,
            "line_ending": self.line_ending,
            "branch": self.branch,
            "ahead_behind": self.ahead_behind,
            "tasks": self.tasks.iter().map(|t| t::task_label(t.kind)).collect::<Vec<_>>(),
            "message": self.message.as_ref().map(|(m, _)| m),
            "operation": self.operation,
            "progress": self.progress_text,
            "message_action": self.message_action,
        })
    }
}

pub fn show(ui: &mut Ui, p: &Palette, model: &StatusModel) -> StatusOut {
    let mut out = StatusOut::default();
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        // À gauche : message, sinon le fichier.
        if let Some((message, is_error)) = &model.message {
            ui.colored_label(if *is_error { p.error } else { p.warn }, message);
            if let Some(label) = &model.message_action {
                if ui.small_button(label).clicked() {
                    out.message_action = true;
                }
            }
        } else if let Some(file) = &model.file {
            ui.label(
                egui::RichText::new(format!("{file}{}", if model.modified { " ●" } else { "" }))
                    .color(p.text_dim)
                    .size(12.5),
            );
        } else {
            ui.label(
                egui::RichText::new(t::STATUS_NO_FILE)
                    .color(p.text_dim)
                    .size(12.5),
            );
        }
        for task in &model.tasks {
            if task_widget(ui, p, task, model.progress_text.as_deref()) {
                out.cancel = Some(task.id);
            }
        }
        if let Some(operation) = &model.operation {
            ui.label(
                egui::RichText::new(operation)
                    .color(p.error)
                    .size(12.5)
                    .strong(),
            );
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            if let Some((line, column)) = model.position {
                ui.label(
                    egui::RichText::new(format!("{line}:{column}"))
                        .color(p.text)
                        .size(12.5),
                );
            }
            if let Some(ending) = model.line_ending {
                ui.label(egui::RichText::new(ending).color(p.text_dim).size(12.5));
            }
            if let Some(encoding) = model.encoding {
                ui.label(egui::RichText::new(encoding).color(p.text_dim).size(12.5));
            }
            if let Some(branch) = &model.branch {
                let mut text = branch.clone();
                if let Some((ahead, behind)) = model.ahead_behind {
                    if ahead > 0 {
                        text.push_str(&format!("  ↑{ahead}"));
                    }
                    if behind > 0 {
                        text.push_str(&format!("  ↓{behind}"));
                    }
                }
                let galley =
                    ui.painter()
                        .layout_no_wrap(text, FontId::proportional(12.5), p.accent);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(galley.size().x + 20.0, 18.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        egui::pos2(rect.min.x + 7.0, rect.center().y),
                        vec2(13.0, 13.0),
                    ),
                    Icon::Branch,
                    p.accent,
                );
                ui.painter().galley(
                    egui::pos2(rect.min.x + 18.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    p.accent,
                );
                response.on_hover_text(t::STATUS_BRANCH_TOOLTIP);
            }
        });
    });
    out
}

/// Rend `true` si la croix d'annulation est cliquée.
fn task_widget(ui: &mut Ui, p: &Palette, task: &TaskInfo, progress_text: Option<&str>) -> bool {
    let mut label = t::task_label(task.kind).to_owned();
    if cancellable(task.kind) {
        if let Some(text) = progress_text {
            label.push_str(&format!(" — {text}"));
        }
    }
    if let Some((done, total)) = task.progress {
        if total > 0 {
            label.push_str(&format!(" {}/{}", done, total));
        } else if done > 0 {
            label.push_str(&format!(" — {}", t::files_count(done)));
        }
    }
    ui.label(egui::RichText::new(label).color(p.text).size(12.5));
    let fraction = match task.progress {
        Some((done, total)) if total > 0 => Some(done as f32 / total as f32),
        _ => None,
    };
    let bar = match fraction {
        Some(f) => egui::ProgressBar::new(f),
        None => egui::ProgressBar::new(0.0).animate(true),
    };
    ui.add(bar.desired_width(90.0).desired_height(6.0).corner_radius(3));
    if !cancellable(task.kind) {
        return false;
    }
    let cancel_rect = ui
        .allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover())
        .0;
    icons::tool_button(
        ui,
        cancel_rect,
        egui::Id::unique(("annuler-tache", task.id)),
        Icon::Close,
        p,
        crate::texts_v1::TIP_CANCEL_TASK,
        true,
    )
}
