//! Disposition des groupes d'éditeurs : l'arbre de divisions, ses séparateurs
//! déplaçables, l'affichage d'un groupe (rangée d'onglets, bandeau, éditeur), le
//! glisser d'un onglet et la sauvegarde de la disposition dans `.juliestorm/`.

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, vec2, Align, Color32, CursorIcon, FontId, Id, Layout, Order, Pos2, Rect, Sense, Ui,
    UiBuilder,
};
use julie_core::{Tasks, TextRange};
use serde_json::{json, Value};

use super::{
    mapped_selections, BannerIntent, EditorClick, Editors, Group, GroupId, Notice, Pending,
    Restoring, TabIntent,
};
use crate::editor::{Decorations, EditorIntent};
use crate::icons::{self, Icon};
use crate::target::{FileRef, MenuRequest, Target, Zone};
use crate::texts as t;
use crate::theme::Palette;

/// Hauteur de la rangée d'onglets.
const TAB_BAR: f32 = 30.0;
/// Épaisseur du séparateur dessiné.
const SEP: f32 = 4.0;
/// Taille minimale d'un groupe : sans elle, un séparateur tiré trop loin rend un groupe
/// inutilisable.
const MIN_WIDTH: f32 = 160.0;
const MIN_HEIGHT: f32 = 110.0;
/// Nom du fichier de disposition dans `.juliestorm/`.
pub const LAYOUT_FILE: &str = "layout.json";

/// Sens d'une division : `Vertical` = deux groupes côte à côte (le séparateur est
/// vertical), `Horizontal` = l'un au-dessus de l'autre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Vertical,
    Horizontal,
}

impl Dir {
    fn name(self) -> &'static str {
        match self {
            Dir::Vertical => "vertical",
            Dir::Horizontal => "horizontal",
        }
    }
}

/// Bord d'un groupe, pour le glisser d'un onglet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// L'arbre des divisions.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(GroupId),
    Split {
        dir: Dir,
        /// Part du premier côté, entre 0 et 1 : gardée au redimensionnement de la fenêtre.
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    pub fn leaves(&self, out: &mut Vec<GroupId>) {
        match self {
            Node::Leaf(g) => out.push(*g),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    fn first_leaf(&self) -> GroupId {
        match self {
            Node::Leaf(g) => *g,
            Node::Split { a, .. } => a.first_leaf(),
        }
    }

    fn last_leaf(&self) -> GroupId {
        match self {
            Node::Leaf(g) => *g,
            Node::Split { b, .. } => b.last_leaf(),
        }
    }

    /// Le groupe qui reprend la place de `id` quand il disparaît.
    pub fn neighbor(&self, id: GroupId) -> Option<GroupId> {
        let Node::Split { a, b, .. } = self else {
            return None;
        };
        if matches!(**a, Node::Leaf(g) if g == id) {
            return Some(b.first_leaf());
        }
        if matches!(**b, Node::Leaf(g) if g == id) {
            return Some(a.last_leaf());
        }
        a.neighbor(id).or_else(|| b.neighbor(id))
    }

    /// L'arbre sans cette feuille (`None` s'il n'en reste aucune).
    pub fn remove(self, id: GroupId) -> Option<Node> {
        match self {
            Node::Leaf(g) => (g != id).then_some(Node::Leaf(g)),
            Node::Split { dir, ratio, a, b } => match (a.remove(id), b.remove(id)) {
                (Some(a), Some(b)) => Some(Node::Split {
                    dir,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }

    /// Remplace la feuille `target` par une division : `target` d'abord, `new` ensuite.
    pub fn split_leaf(&mut self, target: GroupId, new: GroupId, dir: Dir) -> bool {
        self.replace_leaf(target, &mut |leaf| Node::Split {
            dir,
            ratio: 0.5,
            a: Box::new(leaf),
            b: Box::new(Node::Leaf(new)),
        })
    }

    /// Met `new` d'un côté de `target` (glisser sur un bord).
    pub fn insert_beside(&mut self, target: GroupId, new: GroupId, edge: Edge) -> bool {
        let dir = match edge {
            Edge::Left | Edge::Right => Dir::Vertical,
            Edge::Top | Edge::Bottom => Dir::Horizontal,
        };
        let first = matches!(edge, Edge::Left | Edge::Top);
        self.replace_leaf(target, &mut |leaf| {
            let (a, b) = if first {
                (Node::Leaf(new), leaf)
            } else {
                (leaf, Node::Leaf(new))
            };
            Node::Split {
                dir,
                ratio: 0.5,
                a: Box::new(a),
                b: Box::new(b),
            }
        })
    }

    fn replace_leaf(&mut self, target: GroupId, f: &mut dyn FnMut(Node) -> Node) -> bool {
        match self {
            Node::Leaf(g) if *g == target => {
                let leaf = Node::Leaf(*g);
                *self = f(leaf);
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.replace_leaf(target, f) || b.replace_leaf(target, f),
        }
    }

    /// Profondeur maximale (une feuille : 1 niveau).
    pub fn depth(&self) -> usize {
        match self {
            Node::Leaf(_) => 1,
            Node::Split { a, b, .. } => 1 + a.depth().max(b.depth()),
        }
    }
}

/// Un onglet en cours de glisser.
#[derive(Debug, Clone)]
pub(super) struct TabDrag {
    group: GroupId,
    vid: u64,
    title: String,
}

/// Où un onglet glissé atterrirait.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drop {
    /// Dans la rangée d'onglets du groupe, à cet indice (fin si `None`).
    Tabs { group: GroupId, at: Option<usize> },
    /// Un nouveau groupe de ce côté du groupe.
    Edge { group: GroupId, edge: Edge },
}

/// Ce que l'affichage de la zone demande à l'application.
#[derive(Default)]
pub struct Shown {
    pub tab_intents: Vec<TabIntent>,
    pub banner: Option<BannerIntent>,
    pub editor_intents: Vec<EditorIntent>,
}

impl Editors {
    /// Dessine tous les groupes. Un clic dans un groupe le rend actif avant l'affichage.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        active_fill: Color32,
        link: Option<TextRange>,
    ) -> Shown {
        let mut shown = Shown::default();
        let rect = ui.available_rect_before_wrap();
        self.activate_on_press(ui.ctx());
        self.sync_ownership();
        let Some(mut root) = self.root.take() else {
            return shown;
        };
        self.show_node(
            ui,
            &mut root,
            rect,
            Id::unique("editor-splits"),
            p,
            active_fill,
            link,
            &mut shown,
        );
        self.root = Some(root);
        self.show_drag(ui, p, &mut shown);
        shown
    }

    /// Le bouton pressé dans un groupe en fait le groupe actif (sauf s'il tombe sur une
    /// fenêtre flottante, qui garde le focus).
    fn activate_on_press(&mut self, ctx: &egui::Context) {
        let (pressed, pos) = ctx.input(|i| (i.pointer.any_pressed(), i.pointer.interact_pos()));
        let Some(pos) = pressed.then_some(pos).flatten() else {
            return;
        };
        if ctx
            .layer_id_at(pos)
            .is_some_and(|layer| layer.order != Order::Background)
        {
            return;
        }
        let hit = self
            .groups
            .iter()
            .find(|g| g.rect.contains(pos))
            .map(|g| g.id);
        if let Some(group) = hit.filter(|g| *g != self.active_group) {
            self.activate_group(group);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_node(
        &mut self,
        ui: &mut Ui,
        node: &mut Node,
        rect: Rect,
        id: Id,
        p: &Palette,
        fill: Color32,
        link: Option<TextRange>,
        shown: &mut Shown,
    ) {
        match node {
            Node::Leaf(group) => self.show_group(ui, rect, *group, p, fill, link, shown),
            Node::Split { dir, ratio, a, b } => {
                let (total, min_side) = match dir {
                    Dir::Vertical => (rect.width(), MIN_WIDTH),
                    Dir::Horizontal => (rect.height(), MIN_HEIGHT),
                };
                let avail = (total - SEP).max(1.0);
                let min_side = min_side.min(avail * 0.45);
                let split_at = |ratio: f32| (avail * ratio).clamp(min_side, avail - min_side);
                let mut first = split_at(*ratio);
                let sep_rect = |first: f32| match dir {
                    Dir::Vertical => {
                        Rect::from_min_size(rect.min + vec2(first, 0.0), vec2(SEP, rect.height()))
                    }
                    Dir::Horizontal => {
                        Rect::from_min_size(rect.min + vec2(0.0, first), vec2(rect.width(), SEP))
                    }
                };
                let hit = sep_rect(first).expand2(match dir {
                    Dir::Vertical => vec2(2.0, 0.0),
                    Dir::Horizontal => vec2(0.0, 2.0),
                });
                let response = ui.interact(hit, id.with("separator"), Sense::drag());
                if response.hovered() || response.dragged() {
                    ui.ctx().set_cursor_icon(match dir {
                        Dir::Vertical => CursorIcon::ResizeHorizontal,
                        Dir::Horizontal => CursorIcon::ResizeVertical,
                    });
                }
                if response.dragged() {
                    let delta = match dir {
                        Dir::Vertical => response.drag_delta().x,
                        Dir::Horizontal => response.drag_delta().y,
                    };
                    *ratio = ((first + delta) / avail).clamp(0.02, 0.98);
                    first = split_at(*ratio);
                }
                let sep = sep_rect(first);
                let color = if response.dragged() || response.hovered() {
                    p.accent
                } else {
                    p.border
                };
                ui.painter().rect_filled(sep, 0.0, color);
                let (ra, rb) = match dir {
                    Dir::Vertical => (
                        Rect::from_min_size(rect.min, vec2(first, rect.height())),
                        Rect::from_min_max(
                            egui::pos2(rect.min.x + first + SEP, rect.min.y),
                            rect.max,
                        ),
                    ),
                    Dir::Horizontal => (
                        Rect::from_min_size(rect.min, vec2(rect.width(), first)),
                        Rect::from_min_max(
                            egui::pos2(rect.min.x, rect.min.y + first + SEP),
                            rect.max,
                        ),
                    ),
                };
                self.show_node(ui, a, ra, id.with(0), p, fill, link, shown);
                self.show_node(ui, b, rb, id.with(1), p, fill, link, shown);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn show_group(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        group: GroupId,
        p: &Palette,
        fill: Color32,
        link: Option<TextRange>,
        shown: &mut Shown,
    ) {
        let Some(g) = self.group_index(group) else {
            return;
        };
        self.groups[g].rect = rect;
        let is_active = group == self.active_group;
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        child.painter().rect_filled(rect, 0.0, fill);
        if self.groups[g].tabs.is_empty() {
            return;
        }
        self.show_tab_bar(&mut child, group, p, fill, is_active, shown);
        let Some(g) = self.group_index(group) else {
            return;
        };
        self.show_editor(&mut child, g, p, link, is_active, shown);
    }

    fn show_tab_bar(
        &mut self,
        ui: &mut Ui,
        group: GroupId,
        p: &Palette,
        active_fill: Color32,
        group_active: bool,
        shown: &mut Shown,
    ) {
        let titles = self.titles(group);
        let Some(g) = self.group_index(group) else {
            return;
        };
        let reveal = std::mem::take(&mut self.groups[g].reveal_active);
        let active = self.groups[g].active;
        let strip = ui.available_rect_before_wrap();
        ui.painter().rect_filled(
            Rect::from_min_size(strip.min, vec2(strip.width(), TAB_BAR)),
            0.0,
            p.bg_alt,
        );
        let mut menu: Option<MenuRequest> = None;
        let mut rects = Vec::new();
        let mut drag_started: Option<TabDrag> = None;
        let underline = if group_active { p.accent } else { p.text_dim };
        {
            let items = &self.items;
            let tabs = &self.groups[g].tabs;
            egui::ScrollArea::horizontal()
                .id_salt(("editor-tabs", group))
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 1.0;
                        for (i, title) in titles.iter().enumerate() {
                            let Some(tab) = tabs.get(i) else { break };
                            let Some(doc) = items.iter().find(|e| e.id == tab.doc) else {
                                continue;
                            };
                            let is_active = i == active;
                            let modified = doc.document.is_modified();
                            let color = if is_active && group_active {
                                p.text
                            } else if is_active {
                                p.text_dim.gamma_multiply(1.25)
                            } else {
                                p.text_dim
                            };
                            let galley = ui.painter().layout_no_wrap(
                                title.clone(),
                                FontId::proportional(13.5),
                                color,
                            );
                            let width = galley.size().x + 40.0;
                            let (rect, response) = ui
                                .allocate_exact_size(vec2(width, TAB_BAR), Sense::click_and_drag());
                            rects.push(rect);
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
                                    underline,
                                );
                            } else if hovered {
                                painter.rect_filled(rect, 0.0, p.bg);
                            }
                            painter.galley(
                                egui::pos2(
                                    rect.min.x + 12.0,
                                    rect.center().y - galley.size().y / 2.0,
                                ),
                                galley,
                                color,
                            );
                            let close_rect = Rect::from_center_size(
                                egui::pos2(rect.max.x - 14.0, rect.center().y),
                                vec2(16.0, 16.0),
                            );
                            let close = ui.interact(
                                close_rect,
                                Id::unique(("tab-close", group, tab.vid)),
                                Sense::click(),
                            );
                            let painter = ui.painter();
                            if close.hovered() {
                                painter.rect_filled(close_rect, 3.0, p.accent_soft);
                            }
                            if modified && !close.hovered() {
                                icons::paint(painter, close_rect.shrink(2.0), Icon::Dot, p.warn);
                            } else if hovered || is_active || close.hovered() {
                                icons::paint(
                                    painter,
                                    close_rect.shrink(3.5),
                                    Icon::Close,
                                    p.text_dim,
                                );
                            }
                            if reveal && is_active {
                                ui.scroll_to_rect(rect, None);
                            }
                            if response.secondary_clicked() {
                                shown
                                    .tab_intents
                                    .push(TabIntent::Activate { group, index: i });
                                if let Some(path) = doc.path() {
                                    menu = Some(MenuRequest {
                                        target: Target::Files {
                                            zone: Zone::Tab,
                                            files: vec![FileRef {
                                                path: path.to_path_buf(),
                                                is_dir: false,
                                            }],
                                        },
                                        pos: response
                                            .interact_pointer_pos()
                                            .unwrap_or(rect.left_bottom()),
                                        by_keyboard: false,
                                    });
                                }
                            }
                            if response.drag_started() && !close.is_pointer_button_down_on() {
                                drag_started = Some(TabDrag {
                                    group,
                                    vid: tab.vid,
                                    title: title.clone(),
                                });
                            }
                            if close.clicked() || response.middle_clicked() {
                                shown.tab_intents.push(TabIntent::Close { group, index: i });
                            } else if response.clicked() {
                                shown
                                    .tab_intents
                                    .push(TabIntent::Activate { group, index: i });
                            }
                            if let Some(path) = doc.path() {
                                if self.drag.is_none() {
                                    response.on_hover_text(path.display().to_string());
                                }
                            }
                        }
                    });
                });
        }
        self.groups[g].tab_rects = rects;
        if menu.is_some() {
            self.menu_request = menu;
        }
        if drag_started.is_some() {
            self.drag = drag_started;
        }
    }

    fn show_editor(
        &mut self,
        ui: &mut Ui,
        g: usize,
        p: &Palette,
        link: Option<TextRange>,
        is_active: bool,
        shown: &mut Shown,
    ) {
        let group = &mut self.groups[g];
        let Some(tab) = group.tabs.get_mut(group.active) else {
            return;
        };
        let Some(editor) = self.items.iter_mut().find(|e| e.id == tab.doc) else {
            return;
        };
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
                                    if let Some(path) = editor.document.path() {
                                        shown.banner =
                                            Some(BannerIntent::Reload(path.to_path_buf()));
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
        // Une vue qui ne tient pas la sélection vivante dessine la sienne.
        tab.view.paint_selections = if editor.owner == Some(tab.vid) {
            None
        } else {
            Some(match &tab.parked {
                Some(parked) => mapped_selections(&editor.document, parked),
                None => editor.document.selections().clone(),
            })
        };
        let decorations = Decorations {
            link: if is_active { link } else { None },
            ..Decorations::default()
        };
        let output = tab.view.show(ui, &mut editor.document, &decorations);
        if !is_active {
            return;
        }
        shown.editor_intents.extend(output.intents.clone());
        if output.response.secondary_clicked() {
            if let Some(pos) = output.response.interact_pointer_pos() {
                // Dans la sélection : on la garde ; ailleurs, le curseur ira au point cliqué.
                let selection = editor.document.selections().primary();
                let inside_selection = !selection.is_empty()
                    && output.layout.line_at(pos.y).is_some_and(|line| {
                        let buffer = editor.document.buffer();
                        let range = selection.range();
                        let first = buffer.line_of(range.start) as u32;
                        let last = buffer.line_of(range.end) as u32;
                        (first..=last).contains(&line)
                    });
                self.editor_click = Some(EditorClick {
                    pos,
                    inside_selection,
                });
            }
        }
    }

    // ----- Glisser un onglet -----------------------------------------------------------

    /// Où atterrirait un onglet lâché à `pos` ; `None` si rien ne changerait.
    fn drop_target(&self, pos: Pos2, drag: &TabDrag) -> Option<Drop> {
        let group = self.groups.iter().find(|g| g.rect.contains(pos))?;
        let from_tabs = self.groups.iter().find(|g| g.id == drag.group)?.tabs.len();
        if pos.y < group.rect.top() + TAB_BAR {
            let at = group
                .tab_rects
                .iter()
                .position(|r| pos.x < r.center().x)
                .or(Some(group.tabs.len()));
            // Même place : rien à faire.
            if group.id == drag.group {
                let index = group.tabs.iter().position(|t| t.vid == drag.vid)?;
                if at == Some(index) || at == Some(index + 1) {
                    return None;
                }
            }
            return Some(Drop::Tabs {
                group: group.id,
                at,
            });
        }
        let r = group.rect;
        let (fx, fy) = (
            (pos.x - r.left()) / r.width().max(1.0),
            (pos.y - r.top()) / r.height().max(1.0),
        );
        let nearest = [
            (fx, Edge::Left),
            (1.0 - fx, Edge::Right),
            (fy, Edge::Top),
            (1.0 - fy, Edge::Bottom),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
        if nearest.0 < 0.25 {
            if group.id == drag.group && from_tabs < 2 {
                return None;
            }
            return Some(Drop::Edge {
                group: group.id,
                edge: nearest.1,
            });
        }
        if group.id == drag.group {
            return None;
        }
        Some(Drop::Tabs {
            group: group.id,
            at: None,
        })
    }

    fn show_drag(&mut self, ui: &mut Ui, p: &Palette, shown: &mut Shown) {
        let Some(drag) = self.drag.clone() else {
            return;
        };
        let ctx = ui.ctx().clone();
        let (pos, down, escape) = ctx.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.any_down(),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let Some(pos) = pos else {
            self.drag = None;
            return;
        };
        if escape {
            self.drag = None;
            return;
        }
        let target = self.drop_target(pos, &drag);
        let painter = ctx.layer_painter(egui::LayerId::new(
            Order::Foreground,
            Id::unique("tab-drag"),
        ));
        if let Some(target) = target {
            let group_rect = |id: GroupId| self.groups.iter().find(|g| g.id == id).map(|g| g.rect);
            let preview = match target {
                Drop::Tabs { group, .. } => group_rect(group),
                Drop::Edge { group, edge } => group_rect(group).map(|r| match edge {
                    Edge::Left => Rect::from_min_size(r.min, vec2(r.width() / 2.0, r.height())),
                    Edge::Right => Rect::from_min_max(egui::pos2(r.center().x, r.top()), r.max),
                    Edge::Top => Rect::from_min_size(r.min, vec2(r.width(), r.height() / 2.0)),
                    Edge::Bottom => Rect::from_min_max(egui::pos2(r.left(), r.center().y), r.max),
                }),
            };
            if let Some(preview) = preview {
                painter.rect_filled(preview, 4.0, p.accent.gamma_multiply(0.22));
                painter.rect_stroke(
                    preview,
                    4.0,
                    egui::Stroke::new(2.0, p.accent),
                    egui::StrokeKind::Inside,
                );
            }
            if let Drop::Tabs { group, at } = target {
                // Trait d'insertion dans la rangée.
                if let Some(g) = self.groups.iter().find(|g| g.id == group) {
                    let x = match at {
                        Some(i) if i < g.tab_rects.len() => g.tab_rects[i].left(),
                        _ => g.tab_rects.last().map_or(g.rect.left(), |r| r.right()),
                    };
                    painter.line_segment(
                        [
                            egui::pos2(x, g.rect.top() + 3.0),
                            egui::pos2(x, g.rect.top() + TAB_BAR - 3.0),
                        ],
                        egui::Stroke::new(2.0, p.accent),
                    );
                }
            }
        }
        let galley = painter.layout_no_wrap(drag.title.clone(), FontId::proportional(13.5), p.text);
        let ghost = Rect::from_min_size(pos + vec2(14.0, 10.0), galley.size() + vec2(16.0, 8.0));
        painter.rect_filled(ghost, 4.0, p.bg_raised);
        painter.rect_stroke(
            ghost,
            4.0,
            egui::Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
        painter.galley(ghost.min + vec2(8.0, 4.0), galley, p.text);
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        ctx.request_repaint();
        let _ = shown;
        if down {
            return;
        }
        self.drag = None;
        let Some(target) = target else {
            return;
        };
        let Some(from) = self.groups.iter().find(|g| g.id == drag.group) else {
            return;
        };
        let Some(index) = from.tabs.iter().position(|t| t.vid == drag.vid) else {
            return;
        };
        match target {
            Drop::Tabs { group, at } => {
                self.move_tab(drag.group, index, group, at);
            }
            Drop::Edge { group, edge } => {
                self.move_tab_to_edge(drag.group, index, group, edge);
            }
        }
    }

    // ----- Disposition : sauvegarde et restauration -----------------------------------------

    /// La disposition courante, pour `.juliestorm/layout.json` (`None` : aucun onglet).
    pub fn layout_json(&self, project_root: &Path) -> Option<Value> {
        let root = self.root.as_ref()?;
        if self.items.is_empty() {
            return None;
        }
        let order = self.group_order();
        let active_leaf = order
            .iter()
            .position(|g| *g == self.active_group)
            .unwrap_or(0);
        Some(json!({
            "version": 1,
            "active_leaf": active_leaf,
            "root": self.node_json(root, project_root),
        }))
    }

    fn path_text(path: &Path, root: &Path) -> String {
        match path.strip_prefix(root) {
            Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
            Err(_) => path.to_string_lossy().into_owned(),
        }
    }

    fn node_json(&self, node: &Node, root: &Path) -> Value {
        match node {
            Node::Leaf(id) => {
                let Some(group) = self.groups.iter().find(|g| g.id == *id) else {
                    return json!({ "tabs": [] });
                };
                let paths: Vec<String> = group
                    .tabs
                    .iter()
                    .filter_map(|t| {
                        let d = self.doc_index(t.doc)?;
                        Some(Self::path_text(self.items[d].path()?, root))
                    })
                    .collect();
                let active = group
                    .tabs
                    .get(group.active)
                    .and_then(|t| self.doc_index(t.doc))
                    .and_then(|d| self.items[d].path())
                    .map(|p| Self::path_text(p, root));
                json!({ "tabs": paths, "active": active })
            }
            Node::Split { dir, ratio, a, b } => json!({
                "split": dir.name(),
                "ratio": (f64::from(*ratio) * 1000.0).round() / 1000.0,
                "a": self.node_json(a, root),
                "b": self.node_json(b, root),
            }),
        }
    }

    /// Recrée la disposition sauvegardée : les fichiers qui n'existent plus sont
    /// ignorés (et les groupes qui s'en trouvent vides disparaissent). Chargement en
    /// tâche de fond. `false` : rien à restaurer.
    pub fn restore_layout(&mut self, tasks: &Tasks, project_root: &Path, value: &Value) -> bool {
        if !self.items.is_empty() || !self.pending.is_empty() || !self.groups.is_empty() {
            return false;
        }
        let Some(node) = value.get("root") else {
            return false;
        };
        let mut wanted: Vec<(PathBuf, GroupId)> = Vec::new();
        let mut groups: Vec<Group> = Vec::new();
        let Some(root) = self.build_node(node, project_root, &mut groups, &mut wanted) else {
            return false;
        };
        if wanted.is_empty() {
            return false;
        }
        self.groups = groups;
        self.root = Some(root);
        self.active_group = self.group_order().first().copied().unwrap_or(0);
        self.restoring = Some(Restoring {
            active_leaf: value
                .get("active_leaf")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize,
        });
        let mut paths: Vec<PathBuf> = Vec::new();
        for (path, _) in &wanted {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        for path in paths {
            let targets: Vec<GroupId> = wanted
                .iter()
                .filter(|(p, _)| *p == path)
                .map(|(_, g)| *g)
                .collect();
            self.pending.insert(
                path.clone(),
                Pending {
                    goto: None,
                    groups: targets,
                    restore: true,
                },
            );
            julie_text::spawn_load(tasks, path);
        }
        true
    }

    fn build_node(
        &mut self,
        value: &Value,
        root: &Path,
        groups: &mut Vec<Group>,
        wanted: &mut Vec<(PathBuf, GroupId)>,
    ) -> Option<Node> {
        if let Some(kind) = value.get("split").and_then(Value::as_str) {
            let dir = if kind == "horizontal" {
                Dir::Horizontal
            } else {
                Dir::Vertical
            };
            let ratio = value
                .get("ratio")
                .and_then(Value::as_f64)
                .map_or(0.5, |r| r as f32)
                .clamp(0.05, 0.95);
            let a = self.build_node(value.get("a")?, root, groups, wanted);
            let b = self.build_node(value.get("b")?, root, groups, wanted);
            return match (a, b) {
                (Some(a), Some(b)) => Some(Node::Split {
                    dir,
                    ratio,
                    a: Box::new(a),
                    b: Box::new(b),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            };
        }
        let to_path = |text: &str| {
            let path = PathBuf::from(text);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        };
        let tabs: Vec<PathBuf> = value
            .get("tabs")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(to_path)
            .filter(|p| p.is_file())
            .collect();
        if tabs.is_empty() {
            return None;
        }
        let id = self.fresh_id();
        let mut group = Group::new(id);
        group.hint_active = value.get("active").and_then(Value::as_str).map(to_path);
        for path in &tabs {
            wanted.push((path.clone(), id));
        }
        group.hint_order = tabs;
        groups.push(group);
        Some(Node::Leaf(id))
    }

    // ----- État pour le mode scénario -----------------------------------------------------

    fn node_state(&self, node: &Node) -> Value {
        match node {
            Node::Leaf(id) => {
                let index = self
                    .group_order()
                    .iter()
                    .position(|g| g == id)
                    .map(|i| i + 1);
                json!({ "group": index })
            }
            Node::Split { dir, ratio, a, b } => json!({
                "split": dir.name(),
                "ratio": (f64::from(*ratio) * 1000.0).round() / 1000.0,
                "a": self.node_state(a),
                "b": self.node_state(b),
            }),
        }
    }

    /// Les groupes dans l'ordre d'affichage, avec leurs onglets et leur rectangle.
    pub fn groups_state(&self) -> Value {
        let order = self.group_order();
        let list: Vec<Value> = order
            .iter()
            .enumerate()
            .filter_map(|(n, gid)| {
                let group = self.groups.iter().find(|g| g.id == *gid)?;
                let titles = self.titles(*gid);
                let tabs: Vec<Value> = group
                    .tabs
                    .iter()
                    .enumerate()
                    .filter_map(|(i, tab)| {
                        let d = self.doc_index(tab.doc)?;
                        let doc = &self.items[d];
                        let owner = doc.owner == Some(tab.vid);
                        let (line, column) = match (&tab.parked, owner) {
                            (Some(parked), false) => {
                                let sel = mapped_selections(&doc.document, parked).primary();
                                let pos = doc.document.buffer().offset_to_position(sel.head);
                                (pos.line + 1, pos.column + 1)
                            }
                            _ => {
                                let pos = doc.document.cursor_position();
                                (pos.line + 1, pos.column + 1)
                            }
                        };
                        Some(json!({
                            "title": titles.get(i),
                            "path": doc.path().map(|p| p.display().to_string()),
                            "active": i == group.active,
                            "modified": doc.document.is_modified(),
                            "views": self.view_count(doc.id),
                            "line": line,
                            "column": column,
                            "scroll_y": (f64::from(tab.view.scroll_y()) * 10.0).round() / 10.0,
                        }))
                    })
                    .collect();
                Some(json!({
                    "index": n + 1,
                    "active": *gid == self.active_group,
                    "rect": [
                        group.rect.min.x.round(),
                        group.rect.min.y.round(),
                        group.rect.width().round(),
                        group.rect.height().round(),
                    ],
                    "tabs": tabs,
                }))
            })
            .collect();
        json!({
            "count": order.len(),
            "active": order.iter().position(|g| *g == self.active_group).map(|i| i + 1),
            "depth": self.root.as_ref().map_or(0, Node::depth),
            "layout": self.root.as_ref().map(|r| self.node_state(r)),
            "list": list,
            "restoring": self.is_restoring(),
        })
    }

    /// Les onglets du groupe actif, dans la forme historique de la clé `tabs`.
    pub fn active_tabs_state(&self) -> Value {
        let titles = self.titles(self.active_group);
        let Some(group) = self.active_group_ref() else {
            return json!([]);
        };
        Value::Array(
            group
                .tabs
                .iter()
                .enumerate()
                .filter_map(|(i, tab)| {
                    let d = self.doc_index(tab.doc)?;
                    Some((i, tab, &self.items[d]))
                })
                .map(|(i, _tab, editor)| {
                    let doc = &editor.document;
                    let position = doc.cursor_position();
                    json!({
                        "title": titles.get(i),
                        "path": editor.path().map(|p| p.display().to_string()),
                        "active": i == group.active,
                        "modified": doc.is_modified(),
                        "line": position.line + 1,
                        "column": position.column + 1,
                        "notice": editor.notice.map(|n| format!("{n:?}")),
                        "read_only": doc.is_read_only(),
                        "language": format!("{:?}", doc.language()),
                        "lines": doc.buffer().len_lines(),
                        "first_line": doc.buffer().line(0).to_string(),
                    })
                })
                .collect(),
        )
    }
}
