//! Arbre du projet : dossiers listés à la demande (jamais sur le fil de
//! l'interface), couleurs d'état Git, tout au clavier (flèches, Entrée).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, vec2, Color32, EventFilter, Id, Key, Modifiers, Sense, Ui};
use julie_index::IndexService;
use julie_vcs::{FileKind, Status};

use crate::actions::ActionId;
use crate::icons::{self, Icon};
use crate::project::{self, Channel, TreeEntry};
use crate::target::{FileRef, MenuRequest, Target, Zone};
use crate::theme::Palette;

const ROW_HEIGHT: f32 = 22.0;
const INDENT: f32 = 14.0;

struct Listing {
    entries: Vec<TreeEntry>,
    loading: bool,
}

struct Row {
    path: PathBuf,
    name: String,
    depth: usize,
    is_dir: bool,
    ignored: bool,
    expanded: bool,
    loading: bool,
}

pub struct ProjectTree {
    id: Id,
    root: Option<PathBuf>,
    lister: Option<(Arc<IndexService>, Channel)>,
    listings: HashMap<PathBuf, Listing>,
    expanded: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
    /// Sélection multiple (Ctrl+clic, Maj+clic) : tout ce qui est choisi quand ils sont
    /// plusieurs, `selected` compris. Vide : la sélection est `selected` seul.
    multi: BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    /// Touche propre à l'arbre (Suppr, Maj+F6, Alt+Inser) à exécuter par l'application.
    pub key_action: Option<ActionId>,
    /// Menu contextuel demandé à cette image (clic droit).
    pub menu_request: Option<MenuRequest>,
    changes: HashMap<PathBuf, FileKind>,
    dirs_with_changes: HashSet<PathBuf>,
    focus_requested: bool,
    scroll_to_selected: bool,
    last_offset: f32,
    last_viewport: f32,
}

impl ProjectTree {
    pub fn new() -> Self {
        Self {
            id: Id::unique("julie-project-tree"),
            root: None,
            lister: None,
            listings: HashMap::new(),
            expanded: HashSet::new(),
            selected: None,
            multi: BTreeSet::new(),
            anchor: None,
            key_action: None,
            menu_request: None,
            changes: HashMap::new(),
            dirs_with_changes: HashSet::new(),
            focus_requested: false,
            scroll_to_selected: false,
            last_offset: 0.0,
            last_viewport: 400.0,
        }
    }

    /// Le service d'index a été remplacé (réglages) : les prochains listages l'utilisent.
    pub fn set_index(&mut self, index: Arc<IndexService>) {
        if let Some((current, _)) = &mut self.lister {
            *current = index;
        }
    }

    /// Nouveau projet : l'arbre repart de zéro, racine dépliée.
    pub fn reset(&mut self, root: PathBuf, index: Arc<IndexService>, channel: Channel) {
        self.listings.clear();
        self.expanded.clear();
        self.changes.clear();
        self.dirs_with_changes.clear();
        self.selected = Some(root.clone());
        self.multi.clear();
        self.anchor = None;
        self.root = Some(root.clone());
        self.lister = Some((index, channel));
        self.expanded.insert(root.clone());
        self.ensure_listed(&root);
    }

    fn ensure_listed(&mut self, dir: &Path) {
        if self.listings.contains_key(dir) {
            return;
        }
        let (Some((index, channel)), Some(root)) = (&self.lister, &self.root) else {
            return;
        };
        self.listings.insert(
            dir.to_path_buf(),
            Listing {
                entries: Vec::new(),
                loading: true,
            },
        );
        project::spawn_list_dir(
            dir.to_path_buf(),
            root.clone(),
            index.clone(),
            channel.clone(),
        );
    }

    pub fn set_listing(&mut self, dir: PathBuf, entries: Vec<TreeEntry>) {
        self.listings.insert(
            dir,
            Listing {
                entries,
                loading: false,
            },
        );
    }

    /// Relit les dossiers dépliés (Ctrl+Alt+Y).
    pub fn refresh(&mut self) {
        let dirs: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        self.listings.clear();
        for dir in dirs {
            self.ensure_listed(&dir);
        }
    }

    /// État Git : couleur des fichiers et des dossiers qui en contiennent.
    pub fn set_git(&mut self, status: Option<&Status>) {
        self.changes.clear();
        self.dirs_with_changes.clear();
        let Some(status) = status else { return };
        for file in &status.files {
            self.changes.insert(file.path.clone(), file.kind);
            for ancestor in file.path.ancestors().skip(1) {
                if !self.dirs_with_changes.insert(ancestor.to_path_buf()) {
                    break;
                }
            }
        }
    }

    /// Un dossier est en cours de lecture.
    pub fn is_loading(&self) -> bool {
        self.listings.values().any(|l| l.loading)
    }

    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.id))
    }

    /// Déplie tous les dossiers jusqu'à `path` et le sélectionne.
    pub fn expand_path(&mut self, path: &Path) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let mut chain: Vec<PathBuf> = path
            .ancestors()
            .filter(|a| a.starts_with(&root))
            .map(Path::to_path_buf)
            .collect();
        chain.reverse();
        for dir in &chain {
            if dir != path || path.extension().is_none() {
                self.expanded.insert(dir.clone());
                self.ensure_listed(dir);
            }
        }
        self.selected = Some(path.to_path_buf());
        self.scroll_to_selected = true;
    }

    /// Clic avec Ctrl (bascule) ou Maj (plage depuis l'ancre) sur une ligne.
    fn click_selection(&mut self, rows: &[Row], path: &PathBuf, ctrl: bool, shift: bool) {
        if shift {
            let anchor = self
                .anchor
                .clone()
                .or_else(|| self.selected.clone())
                .unwrap_or_else(|| path.clone());
            let from = rows.iter().position(|r| r.path == anchor);
            let to = rows.iter().position(|r| &r.path == path);
            if let (Some(a), Some(b)) = (from, to) {
                self.multi = rows[a.min(b)..=a.max(b)]
                    .iter()
                    .map(|r| r.path.clone())
                    .collect();
            }
        } else if ctrl {
            if self.multi.is_empty() {
                if let Some(current) = &self.selected {
                    self.multi.insert(current.clone());
                }
            }
            if !self.multi.remove(path) {
                self.multi.insert(path.clone());
            }
            self.anchor = Some(path.clone());
        } else {
            self.multi.clear();
            self.anchor = Some(path.clone());
        }
    }

    fn toggle(&mut self, dir: &Path) {
        if self.expanded.contains(dir) {
            self.expanded.remove(dir);
        } else {
            self.expanded.insert(dir.to_path_buf());
            self.ensure_listed(dir);
        }
    }

    fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        let Some(root) = &self.root else { return out };
        out.push(Row {
            path: root.clone(),
            name: root
                .file_name()
                .unwrap_or(root.as_os_str())
                .to_string_lossy()
                .into_owned(),
            depth: 0,
            is_dir: true,
            ignored: false,
            expanded: true,
            loading: self.listings.get(root).is_none_or(|l| l.loading),
        });
        if self.expanded.contains(root) {
            self.push_children(root, 1, &mut out);
        }
        out
    }

    fn push_children(&self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        let Some(listing) = self.listings.get(dir) else {
            return;
        };
        for entry in &listing.entries {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            out.push(Row {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth,
                is_dir: entry.is_dir,
                ignored: entry.ignored,
                expanded,
                loading: expanded && self.listings.get(&entry.path).is_none_or(|l| l.loading),
            });
            if expanded {
                self.push_children(&entry.path, depth + 1, out);
            }
        }
    }

    /// La ligne sélectionnée : chemin et dossier ou non (menu au clavier).
    pub fn selected_ref(&self) -> Option<FileRef> {
        let selected = self.selected.clone()?;
        let is_dir = self.rows().iter().find(|r| r.path == selected)?.is_dir;
        Some(FileRef {
            path: selected,
            is_dir,
        })
    }

    /// Tout ce qui est choisi, dans l'ordre de l'arbre (un seul sans sélection multiple).
    pub fn selection_refs(&self) -> Vec<FileRef> {
        let rows = self.rows();
        if self.multi.len() > 1 {
            return rows
                .iter()
                .filter(|r| self.multi.contains(&r.path))
                .map(|r| FileRef {
                    path: r.path.clone(),
                    is_dir: r.is_dir,
                })
                .collect();
        }
        self.selected_ref().into_iter().collect()
    }

    /// Sélection multiple depuis le mode scénario.
    pub fn select_many(&mut self, paths: Vec<PathBuf>) {
        if let Some(first) = paths.first() {
            self.selected = Some(first.clone());
            self.anchor = Some(first.clone());
        }
        self.multi = paths.into_iter().collect();
    }

    /// Noms (avec indentation) des lignes visibles : pour l'état du mode scénario.
    pub fn visible_names(&self) -> Vec<String> {
        self.rows()
            .iter()
            .map(|r| {
                format!(
                    "{}{}{}",
                    "  ".repeat(r.depth),
                    r.name,
                    if r.is_dir { "/" } else { "" }
                )
            })
            .collect()
    }

    pub fn expanded_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        dirs.sort();
        dirs
    }

    /// Le dossier sélectionné (ou le dossier du fichier sélectionné), portée
    /// proposée à la recherche dans les fichiers.
    pub fn selected_dir(&self) -> Option<PathBuf> {
        let selected = self.selected.as_ref()?;
        if selected.is_dir() {
            Some(selected.clone())
        } else {
            selected.parent().map(Path::to_path_buf)
        }
    }

    fn color_of(&self, row: &Row, p: &Palette) -> Color32 {
        if row.ignored {
            return p.ignored;
        }
        if row.is_dir {
            return if self.dirs_with_changes.contains(&row.path) {
                p.git_modified
            } else {
                p.text
            };
        }
        match self.changes.get(&row.path) {
            Some(FileKind::Modified | FileKind::TypeChanged) => p.git_modified,
            Some(FileKind::Added | FileKind::Renamed | FileKind::Copied) => p.git_added,
            Some(FileKind::Untracked) => p.git_untracked,
            Some(FileKind::Deleted) => p.git_deleted,
            Some(FileKind::Conflicted) => p.git_conflict,
            None => p.text,
        }
    }

    /// Affiche l'arbre ; rend le fichier à ouvrir (double-clic ou Entrée).
    pub fn show(&mut self, ui: &mut Ui, p: &Palette) -> Option<PathBuf> {
        let rows = self.rows();
        let mut to_open: Option<PathBuf> = None;
        let mut toggle: Option<PathBuf> = None;
        let mut select: Option<PathBuf> = None;
        let mut menu: Option<MenuRequest> = None;
        // Sélection multiple : un clic avec Ctrl / Maj, une extension au clavier.
        let mut modified_click: Option<(PathBuf, bool, bool)> = None;
        let mut extend = false;
        let mut clear_multi = false;

        if self.focus_requested {
            self.focus_requested = false;
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        let focused = self.has_focus(ui.ctx());
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.id,
                    EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        tab: false,
                        escape: false,
                    },
                )
            });
            let (up, down, left, right, enter) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                )
            });
            let (delete, rename, new_item, shift_up, shift_down) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Delete),
                    i.consume_key(Modifiers::SHIFT, Key::F6),
                    i.consume_key(Modifiers::ALT, Key::Insert),
                    i.consume_key(Modifiers::SHIFT, Key::ArrowUp),
                    i.consume_key(Modifiers::SHIFT, Key::ArrowDown),
                )
            });
            if delete {
                self.key_action = Some(ActionId::TreeDelete);
            } else if rename {
                self.key_action = Some(ActionId::TreeRename);
            } else if new_item {
                self.key_action = Some(ActionId::TreeNewFile);
            }
            let index = self
                .selected
                .as_ref()
                .and_then(|s| rows.iter().position(|r| &r.path == s))
                .unwrap_or(0);
            if shift_up && index > 0 {
                select = Some(rows[index - 1].path.clone());
                extend = true;
            }
            if shift_down && index + 1 < rows.len() {
                select = Some(rows[index + 1].path.clone());
                extend = true;
            }
            if let Some(row) = rows.get(index) {
                if up && index > 0 {
                    select = Some(rows[index - 1].path.clone());
                }
                if down && index + 1 < rows.len() {
                    select = Some(rows[index + 1].path.clone());
                }
                if right && row.is_dir {
                    if row.expanded {
                        if index + 1 < rows.len() && rows[index + 1].depth > row.depth {
                            select = Some(rows[index + 1].path.clone());
                        }
                    } else {
                        toggle = Some(row.path.clone());
                    }
                }
                if left {
                    if row.is_dir && row.expanded && row.depth > 0 {
                        toggle = Some(row.path.clone());
                    } else if let Some(parent) = row.path.parent() {
                        if rows.iter().any(|r| r.path == parent) {
                            select = Some(parent.to_path_buf());
                        }
                    }
                }
                if enter {
                    if row.is_dir {
                        if row.depth > 0 {
                            toggle = Some(row.path.clone());
                        }
                    } else {
                        to_open = Some(row.path.clone());
                    }
                }
            }
            if select.is_some() {
                self.scroll_to_selected = true;
            }
        }

        let selected_index = self
            .selected
            .as_ref()
            .and_then(|s| rows.iter().position(|r| &r.path == s));
        let mut area = egui::ScrollArea::vertical()
            .id_salt("project-tree-scroll")
            .auto_shrink([false, false]);
        if self.scroll_to_selected {
            self.scroll_to_selected = false;
            if let Some(i) = selected_index {
                let top = i as f32 * ROW_HEIGHT;
                let bottom = top + ROW_HEIGHT;
                if top < self.last_offset {
                    area = area.vertical_scroll_offset(top);
                } else if bottom > self.last_offset + self.last_viewport {
                    area = area.vertical_scroll_offset(bottom - self.last_viewport);
                }
            }
        }

        let output = area.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
            for i in range {
                let row = &rows[i];
                let (rect, response) = ui.allocate_exact_size(
                    vec2(ui.available_width().max(120.0), ROW_HEIGHT),
                    Sense::click(),
                );
                let is_selected =
                    self.selected.as_ref() == Some(&row.path) || self.multi.contains(&row.path);
                let painter = ui.painter();
                if is_selected {
                    let fill = if focused { p.accent_soft } else { p.bg_alt };
                    painter.rect_filled(rect, 0.0, fill);
                } else if response.hovered() {
                    painter.rect_filled(rect, 0.0, p.bg_alt);
                }
                let x = rect.min.x + 6.0 + row.depth as f32 * INDENT;
                let icon_rect = |x: f32| {
                    egui::Rect::from_center_size(
                        egui::pos2(x + 6.0, rect.center().y),
                        vec2(13.0, 13.0),
                    )
                };
                if row.is_dir {
                    let chevron = if row.expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    };
                    icons::paint(painter, icon_rect(x), chevron, p.text_dim);
                }
                let color = self.color_of(row, p);
                let icon = match (row.is_dir, row.expanded) {
                    (true, true) => Icon::FolderOpen,
                    (true, false) => Icon::Folder,
                    _ => Icon::File,
                };
                let icon_color = if row.is_dir && !row.ignored {
                    p.accent
                } else {
                    color
                };
                icons::paint(painter, icon_rect(x + 15.0), icon, icon_color);
                let label = if row.loading && row.expanded {
                    format!("{}  …", row.name)
                } else {
                    row.name.clone()
                };
                painter.text(
                    egui::pos2(x + 34.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(13.5),
                    color,
                );

                if response.clicked() {
                    select = Some(row.path.clone());
                    let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
                    modified_click = Some((row.path.clone(), ctrl, shift));
                    ui.memory_mut(|m| m.request_focus(self.id));
                    let on_chevron = response
                        .interact_pointer_pos()
                        .is_some_and(|pos| pos.x < x + 14.0);
                    if row.is_dir && on_chevron && row.depth > 0 {
                        toggle = Some(row.path.clone());
                    }
                }
                if response.secondary_clicked() {
                    ui.memory_mut(|m| m.request_focus(self.id));
                    // Dans la sélection : le menu agit sur toute la sélection ; ailleurs, la
                    // sélection devient cette ligne.
                    let files = if self.multi.len() > 1 && self.multi.contains(&row.path) {
                        rows.iter()
                            .filter(|r| self.multi.contains(&r.path))
                            .map(|r| FileRef {
                                path: r.path.clone(),
                                is_dir: r.is_dir,
                            })
                            .collect()
                    } else {
                        select = Some(row.path.clone());
                        clear_multi = true;
                        vec![FileRef {
                            path: row.path.clone(),
                            is_dir: row.is_dir,
                        }]
                    };
                    menu = Some(MenuRequest {
                        target: Target::Files {
                            zone: Zone::Tree,
                            files,
                        },
                        pos: response
                            .interact_pointer_pos()
                            .unwrap_or(rect.left_bottom()),
                        by_keyboard: false,
                    });
                }
                if response.double_clicked() {
                    if row.is_dir {
                        if row.depth > 0 {
                            toggle = Some(row.path.clone());
                        }
                    } else {
                        to_open = Some(row.path.clone());
                    }
                }
            }
        });
        // Un vrai widget focalisable : egui reprend le focus d'un identifiant que
        // rien ne déclare à l'image.
        ui.interact(
            output.inner_rect,
            self.id,
            Sense::focusable_noninteractive(),
        );
        self.last_offset = output.state.clamped_offset().y;
        self.last_viewport = output.inner_rect.height();

        if menu.is_some() {
            self.menu_request = menu;
        }
        if clear_multi {
            self.multi.clear();
            self.anchor = select.clone();
        } else if let Some((path, ctrl, shift)) = modified_click {
            self.click_selection(&rows, &path, ctrl, shift);
        } else if extend {
            if let Some(path) = select.clone() {
                if self.anchor.is_none() {
                    self.anchor = self.selected.clone();
                }
                self.click_selection(&rows, &path, false, true);
            }
        } else if select.is_some() {
            // Une flèche ou un clic simple : retour à une sélection simple.
            self.multi.clear();
            self.anchor = select.clone();
        }
        if let Some(path) = select {
            self.selected = Some(path);
        }
        if let Some(path) = toggle {
            self.toggle(&path);
        }
        to_open
    }
}

impl Default for ProjectTree {
    fn default() -> Self {
        Self::new()
    }
}
