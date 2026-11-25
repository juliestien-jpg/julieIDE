//! Onglet Journal de la fenêtre Git : arbre des branches à gauche, liste des
//! commits avec leur graphe en couloirs, détail du commit à droite, filtres
//! (branche, texte, auteur, dates, chemin).
//!
//! Le calcul des couloirs est celui de `julie-vcs` (`GraphBuilder`, pur, page
//! par page) ; ici on ne fait que dessiner. Menu contextuel d'un commit et d'une
//! branche : demandes remplies dans [`LogView::menu_request`].

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, EventFilter, FontId, Id, Key, Modifiers, Rect, RichText,
    Sense, Stroke, TextEdit, Ui,
};
use julie_vcs::{
    BranchList, Change, CommitDetails, CommitFile, CommitSummary, EdgeHalf, GraphBuilder, GraphRow,
    LogPage, LogQuery, RefKind,
};

use crate::branchui::{self, BranchRef, Row};
use crate::gitview::GitIntent;
use crate::icons::{self, Icon};
use crate::kit;
use crate::target::{CommitRef, MenuRequest, Target};
use crate::texts as t;
use crate::theme::Palette;

const ROW: f32 = 22.0;
const FILTER_BAR: f32 = 30.0;
const TREE_WIDTH: f32 = 190.0;
const LANE_WIDTH: f32 = 12.0;

/// Couleurs des couloirs (`GraphRow::color % LANES.len()`).
const LANES: [Color32; 8] = [
    Color32::from_rgb(0x8e, 0x7b, 0xff),
    Color32::from_rgb(0x5f, 0xd3, 0x8d),
    Color32::from_rgb(0xf2, 0xb8, 0x4b),
    Color32::from_rgb(0x6f, 0xa8, 0xff),
    Color32::from_rgb(0xff, 0x7a, 0xb8),
    Color32::from_rgb(0x4f, 0xd1, 0xd9),
    Color32::from_rgb(0xff, 0x6b, 0x7a),
    Color32::from_rgb(0xb5, 0xd9, 0x5b),
];

/// Les filtres du journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    /// Branche (ou « HEAD ») ; `None` : toutes.
    pub branch: Option<String>,
    pub text: String,
    pub author: String,
    /// `AAAA-MM-JJ`
    pub since: String,
    pub until: String,
    pub path: Option<PathBuf>,
}

impl Filters {
    pub fn to_query(&self) -> LogQuery {
        let non_empty = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_owned())
        };
        LogQuery {
            revision: self.branch.clone(),
            all_refs: self.branch.is_none() && self.path.is_none(),
            author: non_empty(&self.author),
            text: non_empty(&self.text),
            since: parse_date(&self.since),
            until: parse_date(&self.until).map(|d| d + 86_399),
            paths: self.path.iter().cloned().collect(),
            follow: self.path.is_some(),
            ..LogQuery::default()
        }
    }
}

/// `AAAA-MM-JJ` → secondes Unix (minuit UTC).
pub fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.trim().split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Jours depuis 1970 (algorithme de Howard Hinnant).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400)
}

pub struct LogView {
    pub commits: Vec<CommitSummary>,
    graph: Vec<GraphRow>,
    builder: GraphBuilder,
    max_lane: usize,
    pub has_more: bool,
    pub loading: bool,
    pub requested: bool,
    pub selected: Option<usize>,
    pub details: Option<CommitDetails>,
    pub details_for: Option<String>,
    pub file_selected: Option<usize>,
    pub filters: Filters,
    /// La requête de la page 0 en cours : les pages d'une autre sont ignorées.
    applied: LogQuery,
    reload_at: Option<Instant>,
    collapsed: HashSet<String>,
    pub focus_request: bool,
    focus_id: Id,
    /// Menu contextuel demandé à cette image (clic droit).
    pub menu_request: Option<MenuRequest>,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            commits: Vec::new(),
            graph: Vec::new(),
            builder: GraphBuilder::new(),
            max_lane: 0,
            has_more: false,
            loading: false,
            requested: false,
            selected: None,
            details: None,
            details_for: None,
            file_selected: None,
            filters: Filters::default(),
            applied: Filters::default().to_query(),
            reload_at: None,
            collapsed: HashSet::new(),
            focus_request: false,
            focus_id: Id::unique("git-journal-focus"),
            menu_request: None,
        }
    }
}

impl LogView {
    /// Repart de zéro : le journal sera relu à l'affichage.
    pub fn reset(&mut self) {
        self.commits.clear();
        self.graph.clear();
        self.builder = GraphBuilder::new();
        self.max_lane = 0;
        self.selected = None;
        self.details = None;
        self.details_for = None;
        self.file_selected = None;
        self.requested = false;
        self.loading = false;
        self.has_more = false;
    }

    /// Change les filtres (recharge).
    pub fn set_filters(&mut self, filters: Filters) {
        self.filters = filters;
        self.reset();
    }

    pub fn on_page(&mut self, page: &LogPage) {
        let normalized = LogQuery {
            skip: 0,
            limit: self.applied.limit,
            ..page.query.clone()
        };
        if normalized != self.applied {
            return; // une page d'un ancien filtre
        }
        self.loading = false;
        if page.query.skip == 0 {
            self.commits.clear();
            self.graph.clear();
            self.builder = GraphBuilder::new();
            self.max_lane = 0;
        }
        if page.query.skip == self.commits.len() {
            let rows = self.builder.push_page(&page.commits);
            self.max_lane = rows
                .iter()
                .map(|r| r.width.max(r.lane + 1))
                .fold(self.max_lane, usize::max);
            self.graph.extend(rows);
            self.commits.extend(page.commits.iter().cloned());
            self.has_more = page.has_more;
        }
    }

    pub fn select(&mut self, index: usize, out: &mut Vec<GitIntent>) -> bool {
        let Some(commit) = self.commits.get(index) else {
            return false;
        };
        let id = commit.id.clone();
        self.selected = Some(index);
        self.focus_request = true;
        self.details = None;
        self.file_selected = None;
        self.details_for = Some(id.clone());
        out.push(GitIntent::LoadCommit(id));
        true
    }

    pub fn commit_ref(&self, index: usize) -> Option<CommitRef> {
        let c = self.commits.get(index)?;
        Some(CommitRef {
            id: c.id.clone(),
            short: c.short_id.clone(),
            subject: c.subject.clone(),
            parents: c.parents.len(),
        })
    }

    pub fn selected_commit(&self) -> Option<CommitRef> {
        self.commit_ref(self.selected?)
    }

    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.focus_id))
    }

    /// Ancre d'un menu ouvert au clavier : sous la ligne choisie (approximation).
    pub fn open_commit_file(&self, index: usize) -> Option<(String, CommitFile)> {
        let details = self.details.as_ref()?;
        let file = details.files.get(index)?;
        Some((details.summary.id.clone(), file.clone()))
    }

    fn reload_if_needed(&mut self, out: &mut Vec<GitIntent>) {
        if let Some(at) = self.reload_at {
            if Instant::now() >= at {
                self.reload_at = None;
                self.reset();
            }
        }
        if !self.requested {
            self.requested = true;
            self.loading = true;
            self.applied = self.filters.to_query();
            out.push(GitIntent::LoadLog(self.applied.clone()));
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        branches: Option<&BranchList>,
        out: &mut Vec<GitIntent>,
    ) {
        self.reload_if_needed(out);
        // Barre de filtres.
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), FILTER_BAR));
        self.show_filters(ui, bar, p);
        let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
        let tree = Rect::from_min_size(body.min, vec2(TREE_WIDTH, body.height()));
        self.show_tree(ui, tree, p, branches);
        ui.painter()
            .vline(tree.right(), body.y_range(), Stroke::new(1.0, p.border));
        let rest = Rect::from_min_max(pos2(tree.right() + 1.0, body.top()), body.max);
        if self.commits.is_empty() {
            let text = if self.loading {
                t::GIT_LOADING
            } else {
                t::LOG_EMPTY
            };
            kit::centered_note(ui, rest, text, p);
            return;
        }
        let left_width = (rest.width() * 0.64).max(300.0).min(rest.width() - 160.0);
        let left = Rect::from_min_size(rest.min, vec2(left_width, rest.height()));
        let right = Rect::from_min_max(pos2(left.right() + 1.0, rest.top()), rest.max);
        ui.painter()
            .vline(left.right(), rest.y_range(), Stroke::new(1.0, p.border));
        self.show_list(ui, left, p, out);
        self.show_details(ui, right, p, out);
    }

    // ----- Filtres ---------------------------------------------------------------

    fn show_filters(&mut self, ui: &mut Ui, bar: Rect, p: &Palette) {
        ui.painter().rect_filled(bar, 0.0, p.bg);
        ui.painter().hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            Stroke::new(1.0, p.border),
        );
        let before = self.filters.clone();
        kit::child(ui, bar.shrink2(vec2(8.0, 3.0)), |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let mut cleared = false;
                if let Some(branch) = self.filters.branch.clone() {
                    cleared |= chip(ui, p, &t::filter_branch(&branch));
                    if cleared {
                        self.filters.branch = None;
                    }
                }
                if let Some(path) = self.filters.path.clone() {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    if chip(ui, p, &t::filter_path(&name)) {
                        self.filters.path = None;
                    }
                }
                for (value, hint, width, salt) in [
                    (
                        &mut self.filters.text,
                        t::FILTER_TEXT_HINT,
                        140.0,
                        "jf-texte",
                    ),
                    (
                        &mut self.filters.author,
                        t::FILTER_AUTHOR_HINT,
                        110.0,
                        "jf-auteur",
                    ),
                    (
                        &mut self.filters.since,
                        t::FILTER_SINCE_HINT,
                        142.0,
                        "jf-depuis",
                    ),
                    (
                        &mut self.filters.until,
                        t::FILTER_UNTIL_HINT,
                        158.0,
                        "jf-jusqua",
                    ),
                ] {
                    ui.add(
                        TextEdit::singleline(value)
                            .hint_text(hint)
                            .desired_width(width)
                            .id_salt(salt),
                    );
                }
            });
        });
        if self.filters != before {
            // On attend la fin de la frappe avant de relire.
            self.reload_at = Some(Instant::now() + Duration::from_millis(350));
            ui.ctx().request_repaint_after(Duration::from_millis(380));
        }
    }

    // ----- Arbre des branches --------------------------------------------------

    fn show_tree(&mut self, ui: &mut Ui, rect: Rect, p: &Palette, branches: Option<&BranchList>) {
        let rows: Vec<Row> = match branches {
            Some(list) => branchui::build_rows(list, "", &self.collapsed)
                .into_iter()
                .skip_while(|r| !matches!(r, Row::Header(h) if h == t::HDR_LOCAL))
                .collect(),
            None => Vec::new(),
        };
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
        let mut set_branch: Option<String> = None;
        let mut toggle: Option<String> = None;
        let mut menu: Option<(BranchRef, egui::Pos2)> = None;
        let mut all = false;
        kit::child(ui, rect, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("journal-arbre")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.set_width(rect.width() - 8.0);
                    for (label, active, key) in [
                        (t::TREE_ALL, self.filters.branch.is_none(), None),
                        (
                            "HEAD",
                            self.filters.branch.as_deref() == Some("HEAD"),
                            Some("HEAD"),
                        ),
                    ] {
                        let (r, response) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 23.0), Sense::click());
                        tree_bg(ui, p, r, active, &response);
                        ui.painter().text(
                            pos2(r.left() + 10.0, r.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(13.5),
                            if active { p.text } else { p.text_dim },
                        );
                        if response.clicked() {
                            match key {
                                Some(k) => set_branch = Some(k.to_owned()),
                                None => all = true,
                            }
                        }
                    }
                    for row in &rows {
                        match row {
                            Row::Header(h) => {
                                let (r, _) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), 24.0),
                                    Sense::hover(),
                                );
                                ui.painter().text(
                                    pos2(r.left() + 8.0, r.center().y + 3.0),
                                    Align2::LEFT_CENTER,
                                    h.to_uppercase(),
                                    FontId::proportional(11.0),
                                    p.text_dim,
                                );
                            }
                            Row::Action(_) => {}
                            Row::Folder {
                                key,
                                label,
                                depth,
                                open,
                            } => {
                                let (r, response) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), 22.0),
                                    Sense::click(),
                                );
                                tree_bg(ui, p, r, false, &response);
                                let x = r.left() + 6.0 + *depth as f32 * 12.0;
                                icons::paint(
                                    ui.painter(),
                                    Rect::from_center_size(
                                        pos2(x + 5.0, r.center().y),
                                        vec2(10.0, 10.0),
                                    ),
                                    if *open {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    },
                                    p.text_dim,
                                );
                                ui.painter().text(
                                    pos2(x + 15.0, r.center().y),
                                    Align2::LEFT_CENTER,
                                    label,
                                    FontId::proportional(13.0),
                                    p.text,
                                );
                                if response.clicked() {
                                    toggle = Some(key.clone());
                                }
                            }
                            Row::Branch {
                                branch,
                                label,
                                depth,
                                current,
                                ..
                            } => {
                                let (r, response) = ui.allocate_exact_size(
                                    vec2(ui.available_width(), 22.0),
                                    Sense::click(),
                                );
                                let active =
                                    self.filters.branch.as_deref() == Some(branch.name.as_str());
                                tree_bg(ui, p, r, active, &response);
                                let x = r.left() + 6.0 + *depth as f32 * 12.0;
                                let clip = ui.painter().with_clip_rect(r);
                                clip.text(
                                    pos2(x + 15.0, r.center().y),
                                    Align2::LEFT_CENTER,
                                    label,
                                    FontId::proportional(13.0),
                                    if *current { p.accent } else { p.text },
                                );
                                if response.clicked() {
                                    set_branch = Some(branch.name.clone());
                                }
                                if response.secondary_clicked() {
                                    menu = Some((
                                        branch.clone(),
                                        response.interact_pointer_pos().unwrap_or(r.left_bottom()),
                                    ));
                                }
                            }
                        }
                    }
                });
        });
        if let Some(key) = toggle {
            if !self.collapsed.remove(&key) {
                self.collapsed.insert(key);
            }
        }
        if all {
            let mut f = self.filters.clone();
            f.branch = None;
            f.path = None;
            self.set_filters(f);
        }
        if let Some(branch) = set_branch {
            let mut f = self.filters.clone();
            f.branch = Some(branch);
            f.path = None;
            self.set_filters(f);
        }
        if let Some((branch, pos)) = menu {
            self.menu_request = Some(MenuRequest {
                target: Target::Branch(branch),
                pos,
                by_keyboard: false,
            });
        }
    }

    // ----- Liste et graphe -------------------------------------------------------

    fn show_list(&mut self, ui: &mut Ui, left: Rect, p: &Palette, out: &mut Vec<GitIntent>) {
        let count = self.commits.len();
        let mut clicked = None;
        let mut want_more = false;
        let focus = self.focus_id;
        ui.interact(left, focus, Sense::click());
        if std::mem::take(&mut self.focus_request) {
            ui.memory_mut(|m| m.request_focus(focus));
        }
        let mut scroll_to: Option<usize> = None;
        if ui.memory(|m| m.has_focus(focus)) {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    focus,
                    EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let page = ((left.height() / ROW) as usize).max(2) - 1;
            let current = self.selected;
            let step =
                |ui: &mut Ui, key: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            let target = if step(ui, Key::ArrowDown) {
                Some(current.map_or(0, |c| (c + 1).min(count - 1)))
            } else if step(ui, Key::ArrowUp) {
                Some(current.map_or(0, |c| c.saturating_sub(1)))
            } else if step(ui, Key::PageDown) {
                Some(current.map_or(0, |c| (c + page).min(count - 1)))
            } else if step(ui, Key::PageUp) {
                Some(current.map_or(0, |c| c.saturating_sub(page)))
            } else if step(ui, Key::Home) {
                Some(0)
            } else if step(ui, Key::End) {
                Some(count - 1)
            } else {
                None
            };
            if let Some(i) = target {
                clicked = Some(i);
                scroll_to = Some(i);
            }
        }
        let graph_w = (self.max_lane as f32 * LANE_WIDTH + 14.0).clamp(26.0, 170.0);
        let view_height = left.height();
        let mut menu: Option<(usize, egui::Pos2)> = None;
        kit::child(ui, left, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut area = egui::ScrollArea::vertical()
                .id_salt("git-journal")
                .auto_shrink([false, false]);
            if let Some(i) = scroll_to {
                area = area.vertical_scroll_offset((i as f32 * ROW - view_height / 2.0).max(0.0));
            }
            area.show_rows(ui, ROW, count, |ui, range| {
                if range.end + 20 >= count && self.has_more && !self.loading {
                    want_more = true;
                }
                for i in range {
                    let selected = self.selected == Some(i);
                    let row = log_row(
                        ui,
                        p,
                        &self.commits[i],
                        self.graph.get(i),
                        graph_w,
                        selected,
                    );
                    if row.clicked {
                        clicked = Some(i);
                        ui.memory_mut(|m| m.request_focus(focus));
                    }
                    if row.secondary {
                        clicked = Some(i);
                        ui.memory_mut(|m| m.request_focus(focus));
                        menu = Some((i, row.pos));
                    }
                }
            });
        });
        if want_more {
            self.loading = true;
            out.push(GitIntent::LoadLog(LogQuery {
                skip: count,
                ..self.applied.clone()
            }));
        }
        if let Some(i) = clicked {
            self.select(i, out);
        }
        if let Some((i, pos)) = menu {
            if let Some(commit) = self.commit_ref(i) {
                self.menu_request = Some(MenuRequest {
                    target: Target::Commit(commit),
                    pos,
                    by_keyboard: false,
                });
            }
        }
    }

    // ----- Détail du commit ------------------------------------------------------

    fn show_details(&mut self, ui: &mut Ui, rect: Rect, p: &Palette, out: &mut Vec<GitIntent>) {
        let Some(index) = self.selected else {
            kit::centered_note(ui, rect, t::LOG_SELECT, p);
            return;
        };
        let Some(summary) = self.commits.get(index).cloned() else {
            return;
        };
        let details = self.details.clone();
        let mut open_file: Option<usize> = None;
        let mut select_file: Option<usize> = None;
        let selected_file = self.file_selected;
        kit::child(ui, rect.shrink2(vec2(10.0, 6.0)), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("git-commit-details")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let message = details
                        .as_ref()
                        .map_or_else(|| summary.subject.clone(), |d| d.message.clone());
                    ui.label(RichText::new(message).color(p.text).size(14.0));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{} <{}>",
                            summary.author.name, summary.author.email
                        ))
                        .color(p.text_dim),
                    );
                    ui.label(
                        RichText::new(kit::format_date(
                            summary.author.time,
                            summary.author.offset_minutes,
                            true,
                        ))
                        .color(p.text_dim),
                    );
                    ui.label(
                        RichText::new(&summary.id)
                            .monospace()
                            .color(p.text_dim)
                            .small(),
                    );
                    ui.add_space(8.0);
                    match &details {
                        None => {
                            ui.label(RichText::new(t::GIT_LOADING).color(p.text_dim));
                        }
                        Some(details) => {
                            ui.label(
                                RichText::new(t::files_count(details.files.len() as u64))
                                    .strong()
                                    .color(p.text),
                            );
                            for (i, file) in details.files.iter().enumerate() {
                                let response =
                                    commit_file_row(ui, p, file, selected_file == Some(i));
                                if response.double_clicked() {
                                    open_file = Some(i);
                                } else if response.clicked() {
                                    select_file = Some(i);
                                }
                            }
                        }
                    }
                });
        });
        if let Some(i) = select_file {
            self.file_selected = Some(i);
        }
        if let Some(i) = open_file {
            self.file_selected = Some(i);
            if let Some((id, file)) = self.open_commit_file(i) {
                out.push(GitIntent::RequestDiff {
                    request: julie_vcs::DiffRequest::commit_file(&id, &file),
                    job: crate::gitview::DiffJob {
                        target: crate::gitview::DiffTarget::Window,
                        token: 0,
                    },
                });
            }
        }
    }

    pub fn state(&self) -> serde_json::Value {
        serde_json::json!({
            "loaded": self.commits.len(),
            "has_more": self.has_more,
            "loading": self.loading,
            "selected": self.selected,
            "subjects": self.commits.iter().take(30).map(|c| c.subject.clone()).collect::<Vec<_>>(),
            "files": self.details.as_ref().map(|d| d.files.iter().map(|f| f.repo_path.clone()).collect::<Vec<_>>()),
            "message": self.details.as_ref().map(|d| d.message.clone()),
            "filters": {
                "branch": self.filters.branch, "text": self.filters.text,
                "author": self.filters.author, "since": self.filters.since,
                "until": self.filters.until,
                "path": self.filters.path.as_ref().map(|p| p.display().to_string()),
            },
            "graph": {
                "lanes": self.max_lane,
                "rows": self.graph.len(),
                "merge_rows": self.graph.iter().zip(&self.commits).filter(|(_, c)| c.is_merge()).count(),
                // Les huit premières lignes : couloir du point et nombre de segments.
                "first": self.graph.iter().take(8).map(|r| format!("lane {} edges {}", r.lane, r.edges.len())).collect::<Vec<_>>(),
            },
        })
    }
}

fn tree_bg(ui: &Ui, p: &Palette, rect: Rect, active: bool, response: &egui::Response) {
    if active {
        ui.painter().rect_filled(rect, 0.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.bg);
    }
}

/// Une pastille de filtre avec sa croix ; `true` si la croix est cliquée.
fn chip(ui: &mut Ui, p: &Palette, text: &str) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(12.5), p.text);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + 32.0, 22.0), Sense::click());
    ui.painter().rect_filled(rect, 11.0, p.accent_soft);
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - galley.size().y / 2.0),
        galley,
        p.text,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(8.0, 8.0)),
        Icon::Close,
        p.text_dim,
    );
    response.on_hover_text(t::TIP_REMOVE_FILTER).clicked()
}

struct RowEvent {
    clicked: bool,
    secondary: bool,
    pos: egui::Pos2,
}

fn log_row(
    ui: &mut Ui,
    p: &Palette,
    commit: &CommitSummary,
    graph: Option<&GraphRow>,
    graph_w: f32,
    selected: bool,
) -> RowEvent {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
    }
    if let Some(row) = graph {
        paint_graph(ui, rect, row, p);
    }
    let color = if commit.is_merge() {
        p.text_dim
    } else {
        p.text
    };
    let hash_w = 64.0;
    let date_w = 112.0;
    let author_w = if rect.width() > 640.0 { 130.0 } else { 0.0 };
    let clip_rect = Rect::from_min_max(
        rect.min,
        pos2(
            rect.right() - hash_w - date_w - author_w - 10.0,
            rect.bottom(),
        ),
    );
    let clip = ui.painter().with_clip_rect(clip_rect);
    let mut x = rect.left() + graph_w + 4.0;
    for label in &commit.refs {
        let (fill, fg) = match label.kind {
            RefKind::Head | RefKind::LocalBranch => (p.accent, Color32::WHITE),
            RefKind::RemoteBranch => (p.bg_raised, p.text),
            RefKind::Tag => (p.warn.gamma_multiply(0.5), p.text),
            RefKind::Other => (p.bg_alt, p.text_dim),
        };
        x += kit::pill(
            ui,
            pos2(x, rect.center().y - 8.0),
            &label.name,
            fill,
            fg,
            clip_rect,
        ) + 4.0;
    }
    clip.text(
        pos2(x + 2.0, rect.center().y),
        Align2::LEFT_CENTER,
        &commit.subject,
        FontId::proportional(13.5),
        color,
    );
    let mut right = rect.right() - 8.0;
    ui.painter().text(
        pos2(right, rect.center().y),
        Align2::RIGHT_CENTER,
        &commit.short_id,
        FontId::monospace(12.0),
        p.text_dim,
    );
    right -= hash_w;
    ui.painter().text(
        pos2(right, rect.center().y),
        Align2::RIGHT_CENTER,
        kit::format_when(commit.author.time, commit.author.offset_minutes),
        FontId::proportional(12.5),
        p.text_dim,
    );
    right -= date_w;
    if author_w > 0.0 {
        ui.painter()
            .with_clip_rect(Rect::from_min_max(
                pos2(right - author_w, rect.top()),
                pos2(right, rect.bottom()),
            ))
            .text(
                pos2(right, rect.center().y),
                Align2::RIGHT_CENTER,
                &commit.author.name,
                FontId::proportional(12.5),
                p.text_dim,
            );
    }
    RowEvent {
        clicked: response.clicked(),
        secondary: response.secondary_clicked(),
        pos: response
            .interact_pointer_pos()
            .unwrap_or(rect.left_bottom()),
    }
}

/// Les couloirs d'une ligne : segments, puis le point du commit.
fn paint_graph(ui: &Ui, rect: Rect, row: &GraphRow, p: &Palette) {
    let x_of = |lane: usize| rect.left() + 10.0 + lane as f32 * LANE_WIDTH;
    let cy = rect.center().y;
    let painter = ui.painter();
    for edge in &row.edges {
        let color = LANES[edge.color as usize % LANES.len()];
        let stroke = Stroke::new(1.7, color);
        let (a, b) = match edge.half {
            EdgeHalf::Top => (pos2(x_of(edge.from), rect.top()), pos2(x_of(edge.to), cy)),
            EdgeHalf::Bottom => (
                pos2(x_of(edge.from), cy),
                pos2(x_of(edge.to), rect.bottom()),
            ),
        };
        painter.line_segment([a, b], stroke);
    }
    let color = LANES[row.color as usize % LANES.len()];
    let center = pos2(x_of(row.lane), cy);
    painter.circle_filled(center, 4.2, p.bg);
    painter.circle_filled(center, 3.4, color);
}

fn commit_file_row(ui: &mut Ui, p: &Palette, file: &CommitFile, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 3.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.bg_alt);
    }
    let color = match file.change {
        Change::Added | Change::Copied => p.git_added,
        Change::Deleted => p.git_deleted,
        _ => p.git_modified,
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(13.0, 13.0)),
        Icon::File,
        color,
    );
    let (name, dir) = match file.repo_path.rsplit_once('/') {
        Some((dir, name)) => (name, dir),
        None => (file.repo_path.as_str(), ""),
    };
    let name_rect = ui.painter().text(
        pos2(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.5),
        color,
    );
    ui.painter().text(
        pos2(name_rect.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        dir,
        FontId::proportional(12.0),
        p.text_dim,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_read_as_utc_midnight() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2000-02-29"), Some(951_782_400));
        assert_eq!(parse_date("2026-11-25"), Some(1_795_564_800));
        assert_eq!(parse_date("hier"), None);
        assert_eq!(parse_date("2026-13-01"), None);
    }

    #[test]
    fn default_filters_ask_for_every_branch() {
        let query = Filters::default().to_query();
        assert!(query.all_refs && query.revision.is_none());
        let filters = Filters {
            branch: Some("main".into()),
            text: " fix ".into(),
            ..Filters::default()
        };
        let query = filters.to_query();
        assert_eq!(query.revision.as_deref(), Some("main"));
        assert_eq!(query.text.as_deref(), Some("fix"));
        assert!(!query.all_refs);
    }

    #[test]
    fn a_file_history_follows_renames_on_one_path() {
        let filters = Filters {
            path: Some(PathBuf::from("a.txt")),
            ..Filters::default()
        };
        let query = filters.to_query();
        assert!(query.follow && query.paths.len() == 1 && !query.all_refs);
    }
}
