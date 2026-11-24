//! JulieStorm — l'application. Seule crate qui assemble les crates métier et egui.
//!
//! Modules : `app` (état et boucle), `actions` (registre), `tabs` (éditeurs à
//! onglets), `tree` (arbre du projet), `tools` (fenêtres d'outils), `status`
//! (barre d'état), `search` (aller au fichier, recherche dans les fichiers,
//! palette), `scenario` (mode d'essai), `theme`, `icons`, `texts` (catalogue).
//! `editor/` est à dev-editeur, `terminal/` à dev-terminal.
//!
//! Lancer : `juliestorm [dossier|fichier…] [--scenario <fichier>]`. Sans
//! argument, le dernier projet est rouvert.

// Pas de console noire derrière la fenêtre en version publiée.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod editor;
mod icons;
mod project;
mod scenario;
mod search;
mod status;
mod tabs;
mod terminal;
mod texts;
mod theme;
mod tools;
mod tree;

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use eframe::egui;

fn main() -> eframe::Result {
    // Sous Windows on impose DirectX 12 : le pilote Vulkan Intel de la machine de
    // développement (igvk64.dll 31.0.101.2140) plante à l'ouverture de la fenêtre.
    // La variable reste prioritaire si quelqu'un la règle à la main.
    #[cfg(windows)]
    if std::env::var_os("WGPU_BACKEND").is_none() {
        std::env::set_var("WGPU_BACKEND", "dx12");
    }

    let options = parse_args(std::env::args_os().skip(1).collect());
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(texts::APP_NAME)
            .with_inner_size([1280.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        texts::APP_NAME,
        native,
        Box::new(move |cc| Ok(Box::new(app::JulieStormApp::new(cc, options)))),
    );
    if scenario::FAILED.load(Ordering::SeqCst) {
        std::process::exit(2);
    }
    result
}

fn parse_args(args: Vec<std::ffi::OsString>) -> app::Options {
    let mut scenario = std::env::var_os("JULIESTORM_SCENARIO").map(PathBuf::from);
    let mut paths = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--scenario" {
            scenario = args.next().map(PathBuf::from);
        } else if let Some(value) = arg.to_string_lossy().strip_prefix("--scenario=") {
            scenario = Some(PathBuf::from(value));
        } else {
            let path = PathBuf::from(arg);
            paths.push(std::path::absolute(&path).unwrap_or(path));
        }
    }
    app::Options { scenario, paths }
}
