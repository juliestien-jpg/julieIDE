//! JulieStorm — l'application. Seule crate qui assemble les crates métier et egui.
//!
//! Pour l'instant : la coquille (arbre du projet, zone d'éditeurs, fenêtre
//! d'outils du bas avec ses onglets). Propriétaire : dev-interface ; l'éditeur
//! (`editor/`) est à dev-editeur.
//!
//! Essai de l'éditeur (lot 3) : `juliestorm <fichier>` ouvre le fichier dans la
//! zone centrale ; `juliestorm <gauche> <droite>` ouvre deux volets côte à côte
//! à défilement synchronisé. Ctrl+S enregistre tous les fichiers modifiés.

// Pas de console noire derrière la fenêtre en version publiée.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod editor;

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use julie_core::bus::Bus;
use julie_core::file::LineEnding;
use julie_core::{Domain, Event, Tasks};
use julie_text::{DiskState, Document, TextEvent};

use editor::{Decorations, EditorView};

fn main() -> eframe::Result {
    // Sous Windows on impose DirectX 12 : le pilote Vulkan Intel de la machine de
    // développement (igvk64.dll 31.0.101.2140) plante à l'ouverture de la fenêtre.
    // La variable reste prioritaire si quelqu'un la règle à la main.
    #[cfg(windows)]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }

    let files: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .map(|path| std::path::absolute(&path).unwrap_or(path))
        .collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("JulieStorm")
            .with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "JulieStorm",
        options,
        Box::new(move |cc| Ok(Box::new(JulieStormApp::new(cc, files)))),
    )
}

/// Onglets de la fenêtre d'outils du bas : les trois piliers du projet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BottomTool {
    #[default]
    Git,
    Tests,
    Search,
}

impl BottomTool {
    const ALL: [BottomTool; 3] = [BottomTool::Git, BottomTool::Tests, BottomTool::Search];

    fn label(self) -> &'static str {
        match self {
            BottomTool::Git => "Git",
            BottomTool::Tests => "Tests",
            BottomTool::Search => "Recherche",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            BottomTool::Git => {
                "Changements locaux, journal, étagère : à venir (julie-vcs, julie-diff)."
            }
            BottomTool::Tests => "Arbre des résultats PHPUnit : à venir (julie-phpunit).",
            BottomTool::Search => "Recherche dans les fichiers : à venir (julie-index).",
        }
    }
}

/// Un fichier ouvert dans la zone centrale.
struct OpenEditor {
    document: Document,
    view: EditorView,
}

struct JulieStormApp {
    bus: Bus,
    tasks: Tasks,
    waker: julie_text::Waker,
    bottom_tool: BottomTool,
    editors: Vec<OpenEditor>,
    /// Ordre d'ouverture demandé (les lectures arrivent dans n'importe quel ordre).
    requested: Vec<PathBuf>,
    active: usize,
    /// Dernier message à montrer (échec d'ouverture, d'enregistrement…).
    message: Option<String>,
    window_focused: bool,
}

impl JulieStormApp {
    fn new(cc: &eframe::CreationContext<'_>, files: Vec<PathBuf>) -> Self {
        let bus = Bus::new();
        // egui ne redessine que sur demande : chaque publication d'un fil de fond
        // réclame une image, sinon l'événement attendrait le prochain mouvement de
        // souris. `request_repaint` est sûr depuis n'importe quel fil.
        let ctx = cc.egui_ctx.clone();
        bus.set_waker(move || ctx.request_repaint());
        let ctx = cc.egui_ctx.clone();
        let waker: julie_text::Waker = Arc::new(move || ctx.request_repaint());
        let tasks = Tasks::new(bus.publisher());
        for path in &files {
            julie_text::spawn_load(&tasks, path.clone());
        }
        Self {
            bus,
            tasks,
            waker,
            bottom_tool: BottomTool::default(),
            editors: Vec::new(),
            requested: files,
            active: 0,
            message: None,
            window_focused: true,
        }
    }

    fn on_text_event(&mut self, event: TextEvent) {
        match event {
            TextEvent::Loaded(loaded) => {
                if let Some(open) = self
                    .editors
                    .iter_mut()
                    .find(|e| e.document.path() == Some(loaded.path.as_path()))
                {
                    open.document.reload(loaded);
                    return;
                }
                let path = loaded.path.clone();
                let mut document = Document::from_loaded(loaded);
                document.enable_syntax(Some(self.waker.clone()));
                let mut view = EditorView::new(&path);
                if self.editors.is_empty() {
                    view.request_focus();
                }
                let rank = |p: &std::path::Path| self.requested.iter().position(|r| r == p);
                let position = self
                    .editors
                    .iter()
                    .position(|e| rank(e.document.path().unwrap_or(&path)) > rank(&path))
                    .unwrap_or(self.editors.len());
                self.editors.insert(position, OpenEditor { document, view });
            }
            TextEvent::LoadFailed { path, error } => {
                self.message = Some(format!("{} : {error}", path.display()));
            }
            TextEvent::Saved(outcome) => {
                if let Some(open) = self
                    .editors
                    .iter_mut()
                    .find(|e| e.document.path() == Some(outcome.path.as_path()))
                {
                    open.document.mark_saved(&outcome);
                }
                self.message = None;
            }
            TextEvent::SaveFailed { path, error } => {
                self.message = Some(format!("{} : {error}", path.display()));
            }
            TextEvent::DiskChecked(states) => {
                for (path, state) in states {
                    let Some(open) = self
                        .editors
                        .iter_mut()
                        .find(|e| e.document.path() == Some(path.as_path()))
                    else {
                        continue;
                    };
                    match state {
                        DiskState::Unchanged => {}
                        DiskState::Changed if !open.document.is_modified() => {
                            julie_text::spawn_load(&self.tasks, path);
                        }
                        DiskState::Changed => {
                            self.message =
                                Some(format!("{} : modifié sur le disque", path.display()));
                        }
                        DiskState::Deleted => {
                            open.document.mark_deleted_on_disk();
                            self.message = Some(format!("{} : supprimé du disque", path.display()));
                        }
                    }
                }
            }
        }
    }

    /// Ctrl+S : tous les documents modifiés.
    fn save_all(&mut self) {
        for open in &mut self.editors {
            if open.document.is_modified() {
                match open.document.save_request() {
                    Ok(request) => {
                        julie_text::spawn_save(&self.tasks, request);
                    }
                    Err(error) => self.message = Some(error.to_string()),
                }
            }
        }
    }

    fn status_text(&self) -> String {
        let Some(open) = self.editors.get(self.active) else {
            return "Aucun projet ouvert".to_owned();
        };
        let document = &open.document;
        let position = document.cursor_position();
        let format = document.format();
        let ending = match format.line_ending {
            LineEnding::Lf => "LF",
            LineEnding::CrLf => "CRLF",
            LineEnding::Cr => "CR",
            LineEnding::Mixed => "mixte",
        };
        let name = document
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!(
            "{name}{}   {}:{}   {}   {ending}",
            if document.is_modified() { " *" } else { "" },
            position.line + 1,
            position.column + 1,
            format.encoding.name(),
        )
    }
}

impl eframe::App for JulieStormApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for event in self.bus.drain() {
            if let Event::Domain(event) = event {
                if event.domain == Domain::Text {
                    if let Ok(event) = event.take::<TextEvent>() {
                        self.on_text_event(event);
                    }
                }
            }
        }

        // Retour du focus : le disque a-t-il bougé ?
        let focused = ui.input(|i| i.focused);
        if focused && !self.window_focused {
            let files = self
                .editors
                .iter()
                .filter_map(|e| Some((e.document.path()?.to_path_buf(), e.document.disk_stamp())))
                .collect();
            julie_text::spawn_check_disk(&self.tasks, files);
        }
        self.window_focused = focused;

        let save = ui.ctx().input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::S,
            ))
        });
        if save {
            self.save_all();
        }

        let status = self.status_text();
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(status);
                if let Some(message) = &self.message {
                    ui.separator();
                    ui.colored_label(ui.visuals().warn_fg_color, message);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(concat!("JulieStorm ", env!("CARGO_PKG_VERSION")));
                });
            });
        });

        egui::Panel::bottom("bottom_tools")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for tool in BottomTool::ALL {
                        ui.selectable_value(&mut self.bottom_tool, tool, tool.label());
                    }
                });
                ui.separator();
                ui.weak(self.bottom_tool.placeholder());
            });

        egui::Panel::left("project_tree")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui.heading("Projet");
                ui.separator();
                ui.weak("Arbre du projet : à venir.");
            });

        egui::CentralPanel::default().show(ui, |ui| {
            let decorations = Decorations::default();
            match self.editors.len() {
                0 => {
                    ui.centered_and_justified(|ui| {
                        ui.weak("Éditeur : ouvrir un fichier en argument (juliestorm <fichier>).");
                    });
                }
                1 => {
                    let open = &mut self.editors[0];
                    open.view.show(ui, &mut open.document, &decorations);
                    self.active = 0;
                }
                _ => {
                    // Essai des volets côte à côte : les deux premiers fichiers.
                    let rect = ui.available_rect_before_wrap();
                    let (panes, _gaps) = editor::split_panes(rect, 2, 24.0);
                    let (left, right) = self.editors.split_at_mut(1);
                    let (a, b) = (&mut left[0], &mut right[0]);
                    let out_a = a
                        .view
                        .show_in_rect(ui, panes[0], &mut a.document, &decorations);
                    let out_b = b
                        .view
                        .show_in_rect(ui, panes[1], &mut b.document, &decorations);
                    if a.view.has_focus(ui.ctx()) {
                        self.active = 0;
                    } else if b.view.has_focus(ui.ctx()) {
                        self.active = 1;
                    }
                    editor::sync_scroll(
                        &mut [&mut a.view, &mut b.view],
                        &[out_a, out_b],
                        |_, _, top| top,
                    );
                }
            }
        });
    }
}
