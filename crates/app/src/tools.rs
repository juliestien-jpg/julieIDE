//! Fenêtres d'outils : bande d'icônes à gauche, arbre du projet à gauche,
//! fenêtre du bas à onglets (Git, Tests, Recherche, Terminal). Git et Tests sont
//! vides en attendant la passe 2 ; leur place est réservée ici.

use eframe::egui::{self, vec2, FontId, Sense, Ui};

use crate::actions::{ActionId, Registry};
use crate::icons::{self, Icon};
use crate::texts as t;
use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomTool {
    #[default]
    Git,
    Tests,
    Search,
    Terminal,
}

impl BottomTool {
    pub const ALL: [BottomTool; 4] = [
        BottomTool::Git,
        BottomTool::Tests,
        BottomTool::Search,
        BottomTool::Terminal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BottomTool::Git => t::TOOL_GIT,
            BottomTool::Tests => t::TOOL_TESTS,
            BottomTool::Search => t::TOOL_SEARCH,
            BottomTool::Terminal => t::TOOL_TERMINAL,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            BottomTool::Git => "git",
            BottomTool::Tests => "tests",
            BottomTool::Search => "search",
            BottomTool::Terminal => "terminal",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            BottomTool::Git => Icon::Branch,
            BottomTool::Tests => Icon::Flask,
            BottomTool::Search => Icon::Search,
            BottomTool::Terminal => Icon::Terminal,
        }
    }

    pub fn action(self) -> ActionId {
        match self {
            BottomTool::Git => ActionId::ToggleGit,
            BottomTool::Tests => ActionId::ToggleTests,
            BottomTool::Search => ActionId::ToggleSearch,
            BottomTool::Terminal => ActionId::ToggleTerminal,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ToolWindows {
    pub project_open: bool,
    pub bottom_open: bool,
    pub bottom: BottomTool,
    /// Fenêtre du bas agrandie (diff, fusion, tests : plus de place).
    pub bottom_tall: bool,
}

impl Default for ToolWindows {
    fn default() -> Self {
        Self {
            project_open: true,
            bottom_open: false,
            bottom: BottomTool::Git,
            bottom_tall: false,
        }
    }
}

/// Bande d'icônes de gauche. Rend les actions à lancer (clics).
pub fn side_strip(
    ui: &mut Ui,
    p: &Palette,
    tools: &ToolWindows,
    registry: &Registry,
) -> Vec<ActionId> {
    let tip = |label: &str, id: ActionId| match registry.shortcut_text(id) {
        Some(shortcut) => format!("{label} ({shortcut})"),
        None => label.to_owned(),
    };
    let mut actions = Vec::new();
    ui.add_space(6.0);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        if icons::button(
            ui,
            Icon::Folder,
            32.0,
            tools.project_open,
            p,
            &tip(t::TOOL_PROJECT, ActionId::ToggleProject),
        )
        .clicked()
        {
            actions.push(ActionId::ToggleProject);
        }
        ui.add_space(8.0);
        for tool in BottomTool::ALL {
            let selected = tools.bottom_open && tools.bottom == tool;
            if icons::button(
                ui,
                tool.icon(),
                32.0,
                selected,
                p,
                &tip(tool.label(), tool.action()),
            )
            .clicked()
            {
                actions.push(tool.action());
            }
        }
    });
    actions
}

/// Ligne d'onglets de la fenêtre du bas. Rend l'onglet cliqué, et si la croix
/// de repli a été cliquée.
pub fn bottom_header(ui: &mut Ui, p: &Palette, tools: &ToolWindows) -> (Option<BottomTool>, bool) {
    let mut clicked = None;
    let mut close = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for tool in BottomTool::ALL {
            let selected = tools.bottom == tool;
            let galley = ui.painter().layout_no_wrap(
                tool.label().to_owned(),
                FontId::proportional(13.5),
                if selected { p.text } else { p.text_dim },
            );
            let (rect, response) =
                ui.allocate_exact_size(vec2(galley.size().x + 36.0, 26.0), Sense::click());
            if selected {
                ui.painter().rect_filled(rect, 0.0, p.bg);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(
                        rect.left_bottom() - vec2(0.0, 2.0),
                        vec2(rect.width(), 2.0),
                    ),
                    0.0,
                    p.accent,
                );
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, p.bg);
            }
            icons::paint(
                ui.painter(),
                egui::Rect::from_center_size(
                    egui::pos2(rect.min.x + 15.0, rect.center().y),
                    vec2(14.0, 14.0),
                ),
                tool.icon(),
                if selected { p.accent } else { p.text_dim },
            );
            ui.painter().galley(
                egui::pos2(rect.min.x + 28.0, rect.center().y - galley.size().y / 2.0),
                galley,
                p.text,
            );
            if response.clicked() {
                clicked = Some(tool);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::button(ui, Icon::Close, 22.0, false, p, t::TOOL_COLLAPSE).clicked() {
                close = true;
            }
        });
    });
    (clicked, close)
}
