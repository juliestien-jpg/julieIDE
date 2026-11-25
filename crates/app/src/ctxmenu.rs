//! Menus contextuels : **un seul mécanisme** pour toutes les zones (arbre du
//! projet, changements Git, onglets, journal, sous-menu d'une branche).
//!
//! Une zone décrit son menu par une liste d'[`Item`] (identifiants d'actions du
//! registre, séparateurs, sous-menus d'un niveau). L'application les résout en
//! [`Entry`] (libellé et raccourci du registre, entrée grisée avec sa raison,
//! confirmation à prévoir pour le destructif), puis ce module les dessine et
//! gère le clavier (Haut/Bas en sautant séparateurs et grisées, Droite ouvre un
//! sous-menu, Gauche le referme, Entrée exécute, Échap ferme un niveau).
//!
//! Aucune règle métier ici : le module rend l'action choisie, `app` l'exécute.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Context, FontId, Frame, Id, Key, Margin, Modifiers, Order,
    Pos2, Rect, Sense, Stroke,
};

use crate::actions::ActionId;
use crate::theme::Palette;

/// Description d'un menu, avant résolution.
#[derive(Debug, Clone)]
pub enum Item {
    Action(ActionId),
    /// Sous-menu : clé stable, libellé, entrées (un niveau au plus).
    Sub(&'static str, String, Vec<Item>),
    Sep,
}

/// Une ligne de menu résolue.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Clé d'action du registre (`vcs.add`) ou du sous-menu (`sub.git`).
    pub key: String,
    pub id: Option<ActionId>,
    pub label: String,
    pub shortcut: Option<String>,
    /// Raison pour laquelle l'entrée est grisée.
    pub disabled: Option<String>,
    /// L'action jette du travail : une confirmation suit.
    pub destructive: bool,
    pub children: Vec<Entry>,
    pub separator: bool,
}

impl Entry {
    pub fn separator() -> Self {
        Self {
            key: String::new(),
            id: None,
            label: String::new(),
            shortcut: None,
            disabled: None,
            destructive: false,
            children: Vec::new(),
            separator: true,
        }
    }

    fn selectable(&self) -> bool {
        !self.separator && self.disabled.is_none()
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "key": self.key,
            "label": self.label,
            "shortcut": self.shortcut,
            "enabled": self.disabled.is_none(),
            "reason": self.disabled,
            "destructive": self.destructive,
            "children": self.children.iter().filter(|e| !e.separator).map(Entry::json).collect::<Vec<_>>(),
        })
    }
}

/// Ce que le menu a décidé à cette image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Close,
    Run(ActionId),
}

pub struct ContextMenu {
    /// Zone qui l'a ouvert (`tree`, `changes`, `tab`, `log`, `branch`…).
    pub zone: &'static str,
    /// Ce sur quoi il agit, pour l'état du mode scénario.
    pub target: String,
    pub anchor: Pos2,
    pub entries: Vec<Entry>,
    pub selected: Option<usize>,
    pub open_sub: Option<usize>,
    pub sub_selected: Option<usize>,
    /// Les rectangles de l'image précédente (clic à l'extérieur).
    rects: Vec<Rect>,
    opened_frame: u64,
}

impl ContextMenu {
    pub fn new(
        zone: &'static str,
        target: String,
        anchor: Pos2,
        entries: Vec<Entry>,
        by_keyboard: bool,
    ) -> Self {
        let mut menu = Self {
            zone,
            target,
            anchor,
            entries,
            selected: None,
            open_sub: None,
            sub_selected: None,
            rects: Vec::new(),
            opened_frame: 0,
        };
        if by_keyboard {
            menu.selected = menu.entries.iter().position(Entry::selectable);
        }
        menu
    }

    /// L'entrée de cette clé, dans le menu ou ses sous-menus (index du parent, index).
    pub fn find(&self, key: &str) -> Option<(Option<usize>, usize, &Entry)> {
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.key == key {
                return Some((None, i, entry));
            }
            for (j, child) in entry.children.iter().enumerate() {
                if child.key == key {
                    return Some((Some(i), j, child));
                }
            }
        }
        None
    }

    /// Ouvre le sous-menu de cette clé (scénario, clavier).
    pub fn expand(&mut self, key: &str) -> bool {
        match self.entries.iter().position(|e| e.key == key) {
            Some(i) if !self.entries[i].children.is_empty() => {
                self.selected = Some(i);
                self.open_sub = Some(i);
                self.sub_selected = self.entries[i].children.iter().position(Entry::selectable);
                true
            }
            _ => false,
        }
    }

    pub fn state(&self) -> serde_json::Value {
        serde_json::json!({
            "zone": self.zone,
            "target": self.target,
            "open_sub": self.open_sub.and_then(|i| self.entries.get(i)).map(|e| e.key.clone()),
            "entries": self.entries.iter().filter(|e| !e.separator).map(Entry::json).collect::<Vec<_>>(),
        })
    }

    fn step(entries: &[Entry], from: Option<usize>, down: bool) -> Option<usize> {
        let n = entries.len();
        if n == 0 {
            return None;
        }
        let mut i = match (from, down) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        for _ in 0..n {
            if entries[i].selectable() {
                return Some(i);
            }
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
        }
        None
    }

    /// Dessine le menu et traite le clavier. À appeler à chaque image tant qu'il est ouvert.
    pub fn show(&mut self, ctx: &Context, p: &Palette) -> Outcome {
        self.opened_frame += 1;
        let (down, up, right, left, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        // Clavier.
        if let Some(sub) = self.open_sub {
            let children = self.entries[sub].children.clone();
            if down {
                self.sub_selected = Self::step(&children, self.sub_selected, true);
            }
            if up {
                self.sub_selected = Self::step(&children, self.sub_selected, false);
            }
            if left || escape {
                self.open_sub = None;
                self.sub_selected = None;
            } else if enter {
                if let Some(child) = self.sub_selected.and_then(|j| children.get(j)) {
                    if let Some(id) = child.id.filter(|_| child.disabled.is_none()) {
                        return Outcome::Run(id);
                    }
                }
            }
        } else {
            if down {
                self.selected = Self::step(&self.entries, self.selected, true);
            }
            if up {
                self.selected = Self::step(&self.entries, self.selected, false);
            }
            if escape {
                return Outcome::Close;
            }
            if right || enter {
                if let Some(i) = self.selected {
                    let entry = self.entries[i].clone();
                    if !entry.children.is_empty() && entry.disabled.is_none() {
                        self.open_sub = Some(i);
                        self.sub_selected = entry.children.iter().position(Entry::selectable);
                    } else if enter {
                        if let Some(id) = entry.id.filter(|_| entry.disabled.is_none()) {
                            return Outcome::Run(id);
                        }
                    }
                }
            }
        }

        // Dessin.
        self.rects.clear();
        let top = draw_level(
            ctx,
            p,
            "ctx-0",
            self.anchor,
            &self.entries,
            self.selected,
            self.open_sub,
        );
        self.rects.push(top.rect);
        if let Some(i) = top.hovered {
            if self.selected != Some(i) {
                self.selected = Some(i);
            }
            let has_children = !self.entries[i].children.is_empty();
            if has_children && self.open_sub != Some(i) {
                self.open_sub = Some(i);
                self.sub_selected = None;
            } else if !has_children && self.open_sub.is_some() && top.moved {
                self.open_sub = None;
                self.sub_selected = None;
            }
        }
        let mut outcome = Outcome::Open;
        if let Some(i) = top.clicked {
            let entry = &self.entries[i];
            if entry.children.is_empty() {
                if let Some(id) = entry.id {
                    outcome = Outcome::Run(id);
                }
            } else {
                self.open_sub = Some(i);
            }
        }
        if let Some(sub) = self.open_sub {
            let parent_rect = top.row_rects.get(sub).copied().unwrap_or(top.rect);
            let children = self.entries[sub].children.clone();
            let level = draw_level(
                ctx,
                p,
                "ctx-1",
                pos2(top.rect.right() - 2.0, parent_rect.top() - 4.0),
                &children,
                self.sub_selected,
                None,
            );
            self.rects.push(level.rect);
            if let Some(j) = level.hovered {
                self.sub_selected = Some(j);
            }
            if let Some(j) = level.clicked {
                if let Some(id) = children[j].id {
                    outcome = Outcome::Run(id);
                }
            }
        }
        // Un clic à l'extérieur ferme sans rien faire.
        if self.opened_frame > 1 && outcome == Outcome::Open {
            let outside = ctx.input(|i| {
                (i.pointer.primary_pressed() || i.pointer.secondary_pressed())
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|pos| !self.rects.iter().any(|r| r.contains(pos)))
            });
            if outside {
                return Outcome::Close;
            }
        }
        outcome
    }
}

struct LevelOut {
    rect: Rect,
    row_rects: Vec<Rect>,
    hovered: Option<usize>,
    clicked: Option<usize>,
    moved: bool,
}

const ROW: f32 = 25.0;

fn draw_level(
    ctx: &Context,
    p: &Palette,
    salt: &'static str,
    pos: Pos2,
    entries: &[Entry],
    selected: Option<usize>,
    open_sub: Option<usize>,
) -> LevelOut {
    let mut out = LevelOut {
        rect: Rect::NOTHING,
        row_rects: Vec::new(),
        hovered: None,
        clicked: None,
        moved: false,
    };
    let area = egui::Area::new(Id::unique(salt))
        .order(Order::Tooltip)
        .fixed_pos(pos)
        .constrain(true)
        .show(ctx, |ui| {
            Frame::new()
                .fill(p.bg_raised)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(8)
                .inner_margin(Margin::same(4))
                .shadow(egui::Shadow {
                    offset: [0, 6],
                    blur: 22,
                    spread: 1,
                    color: Color32::from_black_alpha(110),
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                    let font = FontId::proportional(13.5);
                    let small = FontId::proportional(12.0);
                    let mut label_w = 0.0f32;
                    let mut shortcut_w = 0.0f32;
                    let mut has_sub = false;
                    for e in entries.iter().filter(|e| !e.separator) {
                        label_w = label_w.max(
                            ui.painter()
                                .layout_no_wrap(e.label.clone(), font.clone(), p.text)
                                .size()
                                .x,
                        );
                        if let Some(sc) = &e.shortcut {
                            shortcut_w = shortcut_w.max(
                                ui.painter()
                                    .layout_no_wrap(sc.clone(), small.clone(), p.text)
                                    .size()
                                    .x,
                            );
                        }
                        has_sub |= !e.children.is_empty();
                    }
                    let width = (label_w
                        + if shortcut_w > 0.0 {
                            shortcut_w + 28.0
                        } else {
                            0.0
                        }
                        + if has_sub { 22.0 } else { 0.0 }
                        + 30.0)
                        .max(190.0);
                    ui.set_width(width);
                    for (i, e) in entries.iter().enumerate() {
                        if e.separator {
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(width, 9.0), Sense::hover());
                            out.row_rects.push(rect);
                            ui.painter().hline(
                                rect.x_range().shrink(6.0),
                                rect.center().y,
                                Stroke::new(1.0, p.border),
                            );
                            continue;
                        }
                        let (rect, response) =
                            ui.allocate_exact_size(vec2(width, ROW), Sense::click());
                        out.row_rects.push(rect);
                        let enabled = e.disabled.is_none();
                        let active = selected == Some(i) || open_sub == Some(i);
                        if active && enabled {
                            ui.painter().rect_filled(rect, 5.0, p.accent_soft);
                        }
                        let color = if !enabled {
                            p.ignored
                        } else if e.destructive {
                            p.error
                        } else {
                            p.text
                        };
                        ui.painter().text(
                            pos2(rect.left() + 10.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &e.label,
                            font.clone(),
                            color,
                        );
                        if !e.children.is_empty() {
                            crate::icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(rect.right() - 12.0, rect.center().y),
                                    vec2(10.0, 10.0),
                                ),
                                crate::icons::Icon::ChevronRight,
                                if enabled { p.text_dim } else { p.ignored },
                            );
                        } else if let Some(sc) = &e.shortcut {
                            ui.painter().text(
                                pos2(rect.right() - 10.0, rect.center().y),
                                Align2::RIGHT_CENTER,
                                sc,
                                small.clone(),
                                p.text_dim,
                            );
                        }
                        if response.hovered() {
                            out.hovered = Some(i);
                            out.moved |= ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
                            if let Some(reason) = &e.disabled {
                                response.clone().on_hover_text(reason);
                            }
                        }
                        if response.clicked() && enabled {
                            out.clicked = Some(i);
                        }
                    }
                });
        });
    out.rect = area.response.rect;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, disabled: bool) -> Entry {
        Entry {
            key: key.into(),
            id: None,
            label: key.into(),
            shortcut: None,
            disabled: disabled.then(|| "raison".to_owned()),
            destructive: false,
            children: Vec::new(),
            separator: false,
        }
    }

    #[test]
    fn keyboard_skips_separators_and_disabled_entries_and_wraps() {
        let entries = vec![
            entry("a", false),
            Entry::separator(),
            entry("b", true),
            entry("c", false),
        ];
        assert_eq!(ContextMenu::step(&entries, None, true), Some(0));
        assert_eq!(ContextMenu::step(&entries, Some(0), true), Some(3));
        assert_eq!(ContextMenu::step(&entries, Some(3), true), Some(0));
        assert_eq!(ContextMenu::step(&entries, Some(0), false), Some(3));
        assert_eq!(ContextMenu::step(&entries, None, false), Some(3));
    }

    #[test]
    fn find_looks_into_submenus() {
        let mut parent = entry("sub.git", false);
        parent.children = vec![entry("vcs.add", false), entry("vcs.discard", true)];
        let menu = ContextMenu::new(
            "tree",
            "x".into(),
            Pos2::ZERO,
            vec![entry("file.copy_path", false), parent],
            false,
        );
        assert_eq!(
            menu.find("vcs.discard").map(|(p, j, _)| (p, j)),
            Some((Some(1), 1))
        );
        assert!(menu.find("nope").is_none());
    }
}
