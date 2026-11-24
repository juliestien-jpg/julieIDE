//! JulieStorm — l'application. Seule crate qui assemble les crates métier et egui.
//!
//! Pour l'instant : la coquille vide (arbre du projet, zone d'éditeurs, fenêtre
//! d'outils du bas avec ses onglets). Propriétaire : dev-interface.

// Pas de console noire derrière la fenêtre en version publiée.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use julie_core::bus::Bus;

fn main() -> eframe::Result {
    // Sous Windows on impose DirectX 12 : le pilote Vulkan Intel de la machine de
    // développement (igvk64.dll 31.0.101.2140) plante à l'ouverture de la fenêtre.
    // La variable reste prioritaire si quelqu'un la règle à la main.
    #[cfg(windows)]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("JulieStorm")
            .with_inner_size([1280.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "JulieStorm",
        options,
        Box::new(|cc| Ok(Box::new(JulieStormApp::new(cc)))),
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

struct JulieStormApp {
    bus: Bus,
    bottom_tool: BottomTool,
}

impl JulieStormApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let bus = Bus::new();
        // egui ne redessine que sur demande : chaque publication d'un fil de fond
        // réclame une image, sinon l'événement attendrait le prochain mouvement de
        // souris. `request_repaint` est sûr depuis n'importe quel fil.
        let ctx = cc.egui_ctx.clone();
        bus.set_waker(move || ctx.request_repaint());
        Self {
            bus,
            bottom_tool: BottomTool::default(),
        }
    }
}

impl eframe::App for JulieStormApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Les événements des tâches de fond seront distribués aux vues ici.
        let _events = self.bus.drain();

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Aucun projet ouvert");
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
            ui.centered_and_justified(|ui| {
                ui.weak("Éditeur : à venir (julie-text).");
            });
        });
    }
}
