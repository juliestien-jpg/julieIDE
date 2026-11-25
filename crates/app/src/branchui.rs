//! Les branches à l'écran : le widget de la barre du haut (nom, avance et retard),
//! la fenêtre surgissante (recherche immédiate, actions, récentes, locales et
//! distantes rangées par préfixe), et l'arbre des branches du journal.
//!
//! Aucune règle Git : les données viennent de `julie-vcs` (`BranchList`,
//! `branch_tree`), le sous-menu d'une branche est un menu contextuel ordinaire.

use std::collections::HashSet;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, Context, FontId, Id, Key, Modifiers, Pos2, Rect, RichText,
    Sense, Stroke, TextEdit, Ui,
};
use julie_vcs::{branch_tree, Branch, BranchList, BranchNode};

use crate::actions::ActionId;
use crate::icons::{self, Icon};
use crate::texts_v1 as tv;
use crate::theme::Palette;

/// Une branche désignée (locale ou distante).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRef {
    /// `feature/login` ou `origin/feature/login`.
    pub name: String,
    pub remote: bool,
}

/// Ce que montre le widget de la barre du haut.
pub struct WidgetInfo {
    pub text: String,
    pub ahead: u32,
    pub behind: u32,
    pub kind: WidgetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetKind {
    Normal,
    /// Dépôt sans commit : nom en italique.
    Unborn,
    Detached,
    Operation,
    NoRepository,
}

/// Dessine le widget ; rend sa réponse (clic : ouvrir la fenêtre surgissante).
pub fn widget(ui: &mut Ui, p: &Palette, info: &WidgetInfo, open: bool) -> egui::Response {
    let font = FontId::proportional(13.0);
    let color = match info.kind {
        WidgetKind::NoRepository => p.ignored,
        WidgetKind::Operation => p.error,
        WidgetKind::Detached => p.warn,
        _ => p.text,
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &info.text,
        0.0,
        egui::text::TextFormat {
            font_id: font.clone(),
            color,
            italics: info.kind == WidgetKind::Unborn,
            ..Default::default()
        },
    );
    if info.ahead > 0 {
        job.append(
            &format!("   ↑{}", info.ahead),
            0.0,
            egui::text::TextFormat {
                font_id: font.clone(),
                color: p.ok,
                ..Default::default()
            },
        );
    }
    if info.behind > 0 {
        job.append(
            &format!("  ↓{}", info.behind),
            0.0,
            egui::text::TextFormat {
                font_id: font.clone(),
                color: p.warn,
                ..Default::default()
            },
        );
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let size = vec2(galley.size().x + 44.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if open || response.hovered() {
        ui.painter().rect_filled(rect, 6.0, p.accent_soft);
    } else {
        ui.painter().rect_filled(rect, 6.0, p.bg);
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(13.0, 13.0)),
        Icon::Branch,
        p.accent,
    );
    ui.painter().galley(
        pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.right() - 10.0, rect.center().y), vec2(9.0, 9.0)),
        Icon::ChevronDown,
        p.text_dim,
    );
    response
}

/// Correspondance floue « suite de lettres dans l'ordre » (`fe/lo` trouve
/// `feature/login`), sans casse. Rend les positions des lettres trouvées.
pub fn fuzzy(query: &str, name: &str) -> Option<Vec<usize>> {
    let mut positions = Vec::new();
    let mut chars = name.chars().enumerate();
    for q in query.chars().filter(|c| !c.is_whitespace()) {
        let q = q.to_lowercase().next().unwrap_or(q);
        loop {
            let (i, c) = chars.next()?;
            if c.to_lowercase().next().unwrap_or(c) == q {
                positions.push(i);
                break;
            }
        }
    }
    Some(positions)
}

#[derive(Debug, Clone)]
pub enum Row {
    Header(String),
    Action(ActionId),
    Folder {
        key: String,
        label: String,
        depth: usize,
        open: bool,
    },
    Branch {
        branch: BranchRef,
        label: String,
        depth: usize,
        current: bool,
        favorite: bool,
        upstream: Option<String>,
        ahead: u32,
        behind: u32,
        gone: bool,
        highlight: Vec<usize>,
    },
}

impl Row {
    pub fn selectable(&self) -> bool {
        !matches!(self, Row::Header(_))
    }

    pub fn label(&self, registry: &crate::actions::Registry) -> String {
        match self {
            Row::Header(h) => format!("# {h}"),
            Row::Action(id) => registry.get(*id).label.to_owned(),
            Row::Folder { label, depth, .. } => format!("{}{label}/", "  ".repeat(*depth)),
            Row::Branch {
                label,
                depth,
                current,
                upstream,
                ahead,
                behind,
                gone,
                branch,
                ..
            } => {
                let mut text = format!("{}{}", "  ".repeat(*depth), label);
                if *current {
                    text.push_str(" *");
                }
                if let Some(up) = upstream.as_ref().filter(|_| !branch.remote) {
                    text.push_str(&format!(" [{up}"));
                    if *gone {
                        text.push_str(" amont supprimé");
                    }
                    if *ahead > 0 {
                        text.push_str(&format!(" ↑{ahead}"));
                    }
                    if *behind > 0 {
                        text.push_str(&format!(" ↓{behind}"));
                    }
                    text.push(']');
                }
                text
            }
        }
    }
}

fn is_favorite(name: &str) -> bool {
    name == "main" || name == "master"
}

fn branch_row(b: &Branch, label: String, depth: usize, highlight: Vec<usize>) -> Row {
    Row::Branch {
        branch: BranchRef {
            name: b.name.clone(),
            remote: b.kind == julie_vcs::BranchKind::Remote,
        },
        label,
        depth,
        current: b.current,
        favorite: b.kind == julie_vcs::BranchKind::Local && is_favorite(&b.name),
        upstream: b.tracking.as_ref().map(|t| t.upstream.clone()),
        ahead: b.tracking.as_ref().map_or(0, |t| t.ahead),
        behind: b.tracking.as_ref().map_or(0, |t| t.behind),
        gone: b.tracking.as_ref().is_some_and(|t| t.gone),
        highlight,
    }
}

fn push_tree(
    rows: &mut Vec<Row>,
    nodes: &[BranchNode],
    branches: &[&Branch],
    prefix: &str,
    depth: usize,
    collapsed: &HashSet<String>,
) {
    for node in nodes {
        match node {
            BranchNode::Folder {
                name,
                path,
                children,
            } => {
                let key = format!("{prefix}{path}");
                let open = !collapsed.contains(&key);
                rows.push(Row::Folder {
                    key,
                    label: name.clone(),
                    depth,
                    open,
                });
                if open {
                    push_tree(rows, children, branches, prefix, depth + 1, collapsed);
                }
            }
            BranchNode::Branch { label, index } => {
                rows.push(branch_row(
                    branches[*index],
                    label.clone(),
                    depth,
                    Vec::new(),
                ));
            }
        }
    }
}

/// Les lignes de la fenêtre surgissante pour cette recherche.
pub fn build_rows(list: &BranchList, query: &str, collapsed: &HashSet<String>) -> Vec<Row> {
    let mut rows = Vec::new();
    let query = query.trim();
    if query.is_empty() {
        rows.push(Row::Header(tv::HDR_ACTIONS.to_owned()));
        for id in [
            ActionId::UpdateProject,
            ActionId::GitCommit,
            ActionId::Push,
            ActionId::NewBranch,
        ] {
            rows.push(Row::Action(id));
        }
        if !list.recent.is_empty() {
            rows.push(Row::Header(tv::HDR_RECENT.to_owned()));
            for name in &list.recent {
                if let Some(b) = list.find_local(name) {
                    rows.push(branch_row(b, b.name.clone(), 0, Vec::new()));
                }
            }
        }
    }
    // Les favoris d'abord, puis l'ordre alphabétique.
    let mut local: Vec<&Branch> = list.local.iter().collect();
    local.sort_by_key(|b| (!is_favorite(&b.name), b.name.to_lowercase()));
    let remotes: Vec<&Branch> = list.remote.iter().collect();
    if query.is_empty() {
        rows.push(Row::Header(tv::HDR_LOCAL.to_owned()));
        let names: Vec<&str> = local.iter().map(|b| b.name.as_str()).collect();
        push_tree(
            &mut rows,
            &branch_tree(&names),
            &local,
            "local:",
            0,
            collapsed,
        );
        if !remotes.is_empty() {
            rows.push(Row::Header(tv::HDR_REMOTE.to_owned()));
        }
        for remote in &list.remotes {
            let of: Vec<&Branch> = remotes
                .iter()
                .copied()
                .filter(|b| b.remote.as_deref() == Some(remote.as_str()))
                .collect();
            if of.is_empty() {
                continue;
            }
            let key = format!("remote:{remote}");
            let open = !collapsed.contains(&key);
            rows.push(Row::Folder {
                key,
                label: remote.clone(),
                depth: 0,
                open,
            });
            if open {
                let names: Vec<&str> = of.iter().map(|b| b.short_name.as_str()).collect();
                push_tree(
                    &mut rows,
                    &branch_tree(&names),
                    &of,
                    &format!("remote:{remote}/"),
                    1,
                    collapsed,
                );
            }
        }
    } else {
        let matching = |branches: &[&Branch]| -> Vec<Row> {
            branches
                .iter()
                .filter_map(|b| {
                    fuzzy(query, &b.name).map(|pos| branch_row(b, b.name.clone(), 0, pos))
                })
                .collect()
        };
        let local_rows = matching(&local);
        let remote_rows = matching(&remotes);
        if !local_rows.is_empty() {
            rows.push(Row::Header(tv::HDR_LOCAL.to_owned()));
            rows.extend(local_rows);
        }
        if !remote_rows.is_empty() {
            rows.push(Row::Header(tv::HDR_REMOTE.to_owned()));
            rows.extend(remote_rows);
        }
    }
    rows
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupOut {
    None,
    Close,
    Run(ActionId),
    /// Ouvrir le sous-menu de cette branche, à droite de la ligne.
    Menu {
        branch: BranchRef,
        anchor: Pos2,
    },
}

pub struct BranchPopup {
    pub query: String,
    pub selected: usize,
    pub collapsed: HashSet<String>,
    pub rows: Vec<Row>,
    focus_pending: bool,
    scroll_to_selected: bool,
    last_query: String,
    /// Rectangle de la ligne choisie à la dernière image (ancre du sous-menu).
    rect_of_selected: Option<Rect>,
}

impl BranchPopup {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            collapsed: HashSet::new(),
            rows: Vec::new(),
            focus_pending: true,
            scroll_to_selected: true,
            last_query: String::new(),
            rect_of_selected: None,
        }
    }

    /// Recalcule les lignes après un changement de données ou de recherche.
    pub fn rebuild(&mut self, list: &BranchList) {
        self.rows = build_rows(list, &self.query, &self.collapsed);
        if self.selected >= self.rows.len() || !self.rows[self.selected].selectable() {
            self.selected = self.first_selectable_after(None, true).unwrap_or(0);
        }
    }

    fn first_selectable_after(&self, from: Option<usize>, down: bool) -> Option<usize> {
        let n = self.rows.len();
        if n == 0 {
            return None;
        }
        let mut i = match (from, down) {
            (Some(i), true) => i + 1,
            (Some(i), false) => i.checked_sub(1)?,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        while i < n {
            if self.rows[i].selectable() {
                return Some(i);
            }
            if down {
                i += 1;
            } else {
                i = i.checked_sub(1)?;
            }
        }
        None
    }

    /// Sélectionne la ligne d'une branche (scénario).
    pub fn select_branch(&mut self, name: &str) -> bool {
        match self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Branch { branch, .. } if branch.name == name))
        {
            Some(i) => {
                self.selected = i;
                self.scroll_to_selected = true;
                true
            }
            None => false,
        }
    }

    pub fn selected_branch(&self) -> Option<BranchRef> {
        match self.rows.get(self.selected)? {
            Row::Branch { branch, .. } => Some(branch.clone()),
            _ => None,
        }
    }

    pub fn state(&self, registry: &crate::actions::Registry) -> serde_json::Value {
        serde_json::json!({
            "query": self.query,
            "selected": self.rows.get(self.selected).map(|r| r.label(registry)),
            "rows": self.rows.iter().map(|r| r.label(registry)).collect::<Vec<_>>(),
        })
    }

    /// Dessine la fenêtre ; `anchor` : coin haut-droit sous le widget.
    pub fn show(
        &mut self,
        ctx: &Context,
        p: &Palette,
        list: &BranchList,
        registry: &crate::actions::Registry,
        anchor: Pos2,
        menu_open: bool,
    ) -> PopupOut {
        if self.last_query != self.query {
            self.last_query.clone_from(&self.query);
            self.selected = 0;
            self.rebuild(list);
            self.selected = self
                .rows
                .iter()
                .position(Row::selectable)
                .unwrap_or_default();
            self.scroll_to_selected = true;
        }
        let mut out = PopupOut::None;
        if !menu_open {
            let (down, up, right, enter, escape) = ctx.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if escape {
                return PopupOut::Close;
            }
            if down {
                if let Some(i) = self.first_selectable_after(Some(self.selected), true) {
                    self.selected = i;
                    self.scroll_to_selected = true;
                }
            }
            if up {
                if let Some(i) = self.first_selectable_after(Some(self.selected), false) {
                    self.selected = i;
                    self.scroll_to_selected = true;
                }
            }
            if enter || right {
                match self.rows.get(self.selected).cloned() {
                    Some(Row::Action(id)) if enter => out = PopupOut::Run(id),
                    Some(Row::Folder { key, .. }) => {
                        if !self.collapsed.remove(&key) {
                            self.collapsed.insert(key);
                        }
                        self.rebuild(list);
                    }
                    Some(Row::Branch { .. }) => {
                        out = PopupOut::Menu {
                            branch: self.selected_branch().expect("ligne de branche"),
                            anchor: pos2(anchor.x - 8.0, ctx.viewport_rect().top() + 120.0),
                        };
                    }
                    _ => {}
                }
            }
        }

        let screen = ctx.viewport_rect();
        let width = 430.0;
        let max_h = (screen.height() - 90.0).clamp(240.0, 600.0);
        let pos = pos2((anchor.x - width).max(8.0), anchor.y);
        // Hauteur de la liste : celle de son contenu, au plus ce que l'écran permet.
        let content_h: f32 = self
            .rows
            .iter()
            .map(|r| match r {
                Row::Header(_) => 24.0,
                Row::Action(_) => 26.0,
                Row::Folder { .. } => 24.0,
                Row::Branch { .. } => 25.0,
            })
            .sum::<f32>()
            .max(26.0);
        let list_h = content_h.min(max_h - 70.0);
        let mut clicked_row: Option<(usize, Rect)> = None;
        let mut fetch = false;
        let mut toggle: Option<String> = None;
        let mut selected_rect: Option<Rect> = None;
        let area = egui::Area::new(Id::unique("branches-popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.bg_raised)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(8))
                    .shadow(egui::Shadow {
                        offset: [0, 8],
                        blur: 28,
                        spread: 2,
                        color: Color32::from_black_alpha(110),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            let edit = ui.add(
                                TextEdit::singleline(&mut self.query)
                                    .hint_text(tv::BRANCH_SEARCH_HINT)
                                    .desired_width(width - 40.0)
                                    .margin(vec2(8.0, 6.0))
                                    .id_salt("branches-recherche"),
                            );
                            if self.focus_pending {
                                self.focus_pending = false;
                                edit.request_focus();
                            }
                            let (rect, _) =
                                ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
                            fetch = icons::tool_button(
                                ui,
                                rect,
                                Id::unique("branches-fetch"),
                                Icon::Refresh,
                                p,
                                tv::TIP_FETCH,
                                true,
                            );
                        });
                        ui.add_space(4.0);
                        egui::ScrollArea::vertical()
                            .id_salt("branches-liste")
                            .max_height(list_h)
                            .min_scrolled_height(list_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                if self.rows.is_empty() {
                                    ui.label(
                                        RichText::new(tv::BRANCH_NONE_FOUND).color(p.text_dim),
                                    );
                                }
                                let scroll = std::mem::take(&mut self.scroll_to_selected);
                                for (i, row) in self.rows.iter().enumerate() {
                                    let selected = i == self.selected;
                                    let (rect, response) = draw_row(ui, p, registry, row, selected);
                                    if selected {
                                        selected_rect = Some(rect);
                                        if scroll {
                                            ui.scroll_to_rect(rect, None);
                                        }
                                    }
                                    if response.clicked() {
                                        clicked_row = Some((i, rect));
                                    }
                                    if response.secondary_clicked() {
                                        clicked_row = Some((i, rect));
                                    }
                                    if let Row::Folder { key, .. } = row {
                                        if response.clicked() {
                                            toggle = Some(key.clone());
                                        }
                                    }
                                }
                            });
                    });
            });
        let popup_rect = area.response.rect;
        self.rect_of_selected = selected_rect;
        if fetch {
            return PopupOut::Run(ActionId::Fetch);
        }
        if let Some(key) = toggle {
            if !self.collapsed.remove(&key) {
                self.collapsed.insert(key);
            }
            self.rebuild(list);
        } else if let Some((i, rect)) = clicked_row {
            self.selected = i;
            match &self.rows[i] {
                Row::Action(id) => return PopupOut::Run(*id),
                Row::Branch { branch, .. } => {
                    return PopupOut::Menu {
                        branch: branch.clone(),
                        anchor: pos2(rect.right() - 6.0, rect.top()),
                    }
                }
                _ => {}
            }
        }
        if !menu_open && out == PopupOut::None {
            let outside = ctx.input(|i| {
                i.pointer.primary_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|pos| !popup_rect.contains(pos))
            });
            if outside {
                return PopupOut::Close;
            }
        }
        // Le sous-menu ouvert au clavier se place à droite de la ligne choisie.
        if let PopupOut::Menu { branch, .. } = &out {
            let anchor = self.rect_of_selected.map_or(
                pos2(popup_rect.right() - 6.0, popup_rect.top() + 60.0),
                |r| pos2(r.right() - 6.0, r.top()),
            );
            return PopupOut::Menu {
                branch: branch.clone(),
                anchor,
            };
        }
        out
    }
}

impl Default for BranchPopup {
    fn default() -> Self {
        Self::new()
    }
}

fn draw_row(
    ui: &mut Ui,
    p: &Palette,
    registry: &crate::actions::Registry,
    row: &Row,
    selected: bool,
) -> (Rect, egui::Response) {
    match row {
        Row::Header(text) => {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            ui.painter().text(
                pos2(rect.left() + 6.0, rect.center().y + 3.0),
                Align2::LEFT_CENTER,
                text.to_uppercase(),
                FontId::proportional(11.0),
                p.text_dim,
            );
            (rect, response)
        }
        Row::Action(id) => {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
            row_bg(ui, p, rect, selected, &response);
            let action = registry.get(*id);
            ui.painter().text(
                pos2(rect.left() + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                action.label,
                FontId::proportional(13.5),
                p.text,
            );
            if let Some(shortcut) = registry.shortcut_text(*id) {
                ui.painter().text(
                    pos2(rect.right() - 10.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    shortcut,
                    FontId::proportional(12.0),
                    p.text_dim,
                );
            }
            (rect, response)
        }
        Row::Folder {
            label, depth, open, ..
        } => {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            row_bg(ui, p, rect, selected, &response);
            let x = rect.left() + 8.0 + *depth as f32 * 16.0;
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(x + 5.0, rect.center().y), vec2(11.0, 11.0)),
                if *open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                p.text_dim,
            );
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(x + 22.0, rect.center().y), vec2(13.0, 13.0)),
                Icon::Folder,
                p.accent,
            );
            ui.painter().text(
                pos2(x + 34.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(13.5),
                p.text,
            );
            (rect, response)
        }
        Row::Branch {
            branch,
            label,
            depth,
            current,
            favorite,
            upstream,
            ahead,
            behind,
            gone,
            highlight,
        } => {
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 25.0), Sense::click());
            row_bg(ui, p, rect, selected, &response);
            let x = rect.left() + 10.0 + *depth as f32 * 16.0;
            icons::paint(
                ui.painter(),
                Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(13.0, 13.0)),
                Icon::Branch,
                if *current { p.accent } else { p.text_dim },
            );
            let mut text_x = x + 20.0;
            if *favorite {
                paint_star(ui, pos2(text_x + 5.0, rect.center().y), p.warn);
                text_x += 16.0;
            }
            let font = FontId::proportional(13.5);
            let color = if *current { p.accent } else { p.text };
            let name_rect = if highlight.is_empty() {
                ui.painter().text(
                    pos2(text_x, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    font,
                    color,
                )
            } else {
                let ranges: Vec<(usize, usize)> = highlight.iter().map(|&c| (c, c + 1)).collect();
                let job = crate::search::highlighted_job(
                    label, &ranges, font, color, p.spark_fg, p.spark_bg,
                );
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let r = Rect::from_min_size(
                    pos2(text_x, rect.center().y - galley.size().y / 2.0),
                    galley.size(),
                );
                ui.painter().galley(r.min, galley, color);
                r
            };
            let mut right = rect.right() - 8.0;
            if *behind > 0 {
                let r = ui.painter().text(
                    pos2(right, rect.center().y),
                    Align2::RIGHT_CENTER,
                    format!("↓{behind}"),
                    FontId::proportional(12.5),
                    p.warn,
                );
                right = r.left() - 8.0;
            }
            if *ahead > 0 {
                let r = ui.painter().text(
                    pos2(right, rect.center().y),
                    Align2::RIGHT_CENTER,
                    format!("↑{ahead}"),
                    FontId::proportional(12.5),
                    p.ok,
                );
                right = r.left() - 8.0;
            }
            if *gone {
                let r = ui.painter().text(
                    pos2(right, rect.center().y),
                    Align2::RIGHT_CENTER,
                    tv::UPSTREAM_GONE_MARK,
                    FontId::proportional(12.0),
                    p.warn,
                );
                right = r.left() - 8.0;
            }
            if let Some(upstream) = upstream.as_ref().filter(|_| !branch.remote) {
                let clip = ui.painter().with_clip_rect(Rect::from_min_max(
                    pos2(name_rect.right() + 8.0, rect.top()),
                    pos2(right, rect.bottom()),
                ));
                clip.text(
                    pos2(right, rect.center().y),
                    Align2::RIGHT_CENTER,
                    upstream,
                    FontId::proportional(12.0),
                    p.ignored,
                );
            }
            (rect, response)
        }
    }
}

fn row_bg(ui: &Ui, p: &Palette, rect: Rect, selected: bool, response: &egui::Response) {
    if selected {
        ui.painter().rect_filled(rect, 5.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, p.bg_alt);
    }
}

/// Une petite étoile pleine (favori).
fn paint_star(ui: &Ui, center: Pos2, color: Color32) {
    let mut points = Vec::new();
    for i in 0..10 {
        let angle = std::f32::consts::PI * (i as f32) / 5.0 - std::f32::consts::FRAC_PI_2;
        let r = if i % 2 == 0 { 6.0 } else { 2.6 };
        points.push(pos2(center.x + r * angle.cos(), center.y + r * angle.sin()));
    }
    ui.painter()
        .add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_vcs::{BranchCommit, BranchKind, Tracking};

    fn local(name: &str, current: bool, tracking: Option<(&str, u32, u32, bool)>) -> Branch {
        Branch {
            name: name.into(),
            full_ref: format!("refs/heads/{name}"),
            kind: BranchKind::Local,
            remote: None,
            short_name: name.into(),
            current,
            tracking: tracking.map(|(up, ahead, behind, gone)| Tracking {
                upstream: up.into(),
                ahead,
                behind,
                gone,
            }),
            last_commit: BranchCommit {
                id: "0".repeat(40),
                subject: "x".into(),
                author: "a".into(),
                time: 0,
            },
        }
    }

    fn remote(name: &str) -> Branch {
        let (r, short) = name.split_once('/').unwrap();
        Branch {
            name: name.into(),
            full_ref: format!("refs/remotes/{name}"),
            kind: BranchKind::Remote,
            remote: Some(r.into()),
            short_name: short.into(),
            current: false,
            tracking: None,
            last_commit: BranchCommit {
                id: "0".repeat(40),
                subject: "x".into(),
                author: "a".into(),
                time: 0,
            },
        }
    }

    fn list() -> BranchList {
        BranchList {
            current: Some("main".into()),
            local: vec![
                local("bugfix/crash", false, None),
                local(
                    "feature/login",
                    false,
                    Some(("origin/feature/login", 0, 2, false)),
                ),
                local("feature/search", false, None),
                local("main", true, Some(("origin/main", 1, 0, false))),
            ],
            remote: vec![remote("origin/feature/login"), remote("origin/main")],
            remotes: vec!["origin".into()],
            recent: vec!["feature/login".into()],
            ..BranchList::default()
        }
    }

    #[test]
    fn fuzzy_matches_the_full_name_in_order() {
        assert_eq!(fuzzy("fe/lo", "feature/login"), Some(vec![0, 1, 7, 8, 9]));
        assert!(fuzzy("fe/se", "feature/search").is_some());
        assert!(fuzzy("fe/se", "feature/login").is_none());
        assert!(fuzzy("FE/LO", "feature/login").is_some());
    }

    #[test]
    fn rows_group_by_prefix_with_a_single_branch_folded_and_favorites_first() {
        let rows = build_rows(&list(), "", &HashSet::new());
        let registry = crate::actions::Registry::new();
        let labels: Vec<String> = rows.iter().map(|r| r.label(&registry)).collect();
        let local = labels.iter().position(|l| l == "# Locales").unwrap();
        assert!(labels[local + 1].starts_with("main"), "{labels:?}");
        assert!(labels.contains(&"bugfix/crash".to_owned()), "{labels:?}");
        assert!(labels.contains(&"feature/".to_owned()), "{labels:?}");
        // La distante est rangée sous son distant.
        assert!(labels.contains(&"origin/".to_owned()), "{labels:?}");
    }

    #[test]
    fn filtering_flattens_and_keeps_only_matches() {
        let rows = build_rows(&list(), "fe/se", &HashSet::new());
        let names: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Branch { branch, .. } => Some(branch.name.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["feature/search"]);
    }

    #[test]
    fn recent_branches_follow_the_actions() {
        let rows = build_rows(&list(), "", &HashSet::new());
        let recent = rows
            .iter()
            .position(|r| matches!(r, Row::Header(h) if h == "Récentes"))
            .unwrap();
        assert!(
            matches!(&rows[recent + 1], Row::Branch { branch, .. } if branch.name == "feature/login")
        );
    }
}
