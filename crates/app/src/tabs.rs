//! Zone d'éditeurs : documents ouverts, groupes d'onglets disposés en arbre de
//! divisions (split view), enregistrer, bandeau « changé sur le disque », saut à
//! une ligne.
//!
//! Le contenu et l'édition appartiennent à `julie-text` et au composant
//! `editor/` ; ce module les range.
//!
//! # Documents et vues
//!
//! - [`Editors::items`] liste les **documents** (un par fichier ouvert, quel que soit
//!   le nombre de vues) : modification, enregistrement, rechargement après un
//!   changement sur le disque et question de fermeture ne s'appliquent donc qu'une
//!   fois.
//! - Un [`Group`] est une rangée d'onglets ; chaque onglet ([`TabView`]) est une
//!   **vue** d'un document, avec son propre [`EditorView`] (défilement, composition).
//!   Un document peut avoir plusieurs vues, dans des groupes différents ou non.
//! - La disposition des groupes est un arbre ([`Node`]) : voir `tabs/splits.rs`
//!   (arbre, séparateurs, glisser d'un onglet, sauvegarde dans `.juliestorm/`).
//!
//! # Curseur de chaque vue
//!
//! Les sélections vivent dans le `Document` (contrat `julie-text`), pas dans la
//! vue. Pour que deux vues d'un même document aient chacune **leur** curseur, le
//! document ne garde la sélection « vivante » que de la vue qui a le focus
//! (`OpenEditor::owner`) ; les autres vues rangent la leur (`TabView::parked`, recalée
//! à chaque modification par `Buffer::map_offset_since`) et la dessinent par
//! `EditorView::paint_selections`. Le changement de vue active échange les deux.

mod splits;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Rect};
use julie_core::{Tasks, TextPosition};
use julie_index::UnsavedFiles;
use julie_text::{Bias, DiskState, Document, LoadedFile, SaveOutcome, Selection, Selections};

use crate::editor::EditorView;
use crate::target::MenuRequest;
use crate::texts as t;
use crate::theme;

pub use splits::{Dir, Edge, Node, LAYOUT_FILE};

pub type GroupId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Modifié sur le disque alors que l'onglet est modifié.
    Changed,
    Deleted,
}

/// Un document ouvert (un par fichier) : son contenu et ce qui le concerne, pas son
/// affichage.
pub struct OpenEditor {
    pub id: u64,
    pub document: Document,
    pub notice: Option<Notice>,
    /// « Garder les miennes » : le bandeau ne revient pas avant rechargement ou
    /// enregistrement.
    keep_mine: bool,
    confirm_reload: bool,
    /// La vue qui tient la sélection vivante du document.
    owner: Option<u64>,
}

impl OpenEditor {
    pub fn path(&self) -> Option<&Path> {
        self.document.path()
    }
}

/// Sélections rangées d'une vue qui n'a pas le focus.
struct Parked {
    selections: Selections,
    version: u64,
}

/// Un onglet : une vue d'un document.
pub struct TabView {
    pub doc: u64,
    /// Identifiant de la vue (unique dans toute la zone).
    pub vid: u64,
    pub view: EditorView,
    parked: Option<Parked>,
}

/// Un groupe d'éditeurs : une rangée d'onglets et l'éditeur de l'onglet actif.
pub struct Group {
    pub id: GroupId,
    pub tabs: Vec<TabView>,
    pub active: usize,
    /// Document de l'onglet actif d'avant (Ctrl+Tab).
    previous: Option<u64>,
    /// Rectangle de la dernière image (clic dans un groupe, glisser d'un onglet).
    pub rect: Rect,
    /// Rectangles des onglets de la dernière image.
    tab_rects: Vec<Rect>,
    reveal_active: bool,
    /// Restauration : ordre voulu des onglets et onglet actif, tant que tout n'est pas chargé.
    hint_order: Vec<PathBuf>,
    hint_active: Option<PathBuf>,
}

impl Group {
    fn new(id: GroupId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active: 0,
            previous: None,
            rect: Rect::NOTHING,
            tab_rects: Vec::new(),
            reveal_active: false,
            hint_order: Vec::new(),
            hint_active: None,
        }
    }
}

/// Où sauter après l'ouverture : position (base zéro) et fin de sélection éventuelle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Goto {
    pub position: TextPosition,
    pub end: Option<TextPosition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabIntent {
    Activate { group: GroupId, index: usize },
    Close { group: GroupId, index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerIntent {
    Reload(PathBuf),
}

/// Un fichier en cours de chargement : où le montrer, où sauter.
struct Pending {
    goto: Option<Goto>,
    /// Groupes qui recevront un onglet (vide : le groupe actif).
    groups: Vec<GroupId>,
    restore: bool,
}

/// Disposition en cours de restauration : numéro de la feuille active.
struct Restoring {
    active_leaf: usize,
}

#[derive(Default)]
pub struct Editors {
    /// Les documents ouverts, un par fichier.
    pub items: Vec<OpenEditor>,
    /// Les groupes (ordre de création ; l'ordre d'affichage est celui de l'arbre).
    pub groups: Vec<Group>,
    /// L'arbre des divisions (`None` : aucun groupe).
    pub root: Option<Node>,
    pub active_group: GroupId,
    next_id: u64,
    dark: bool,
    pending: HashMap<PathBuf, Pending>,
    restoring: Option<Restoring>,
    /// Onglets à fermer dès que leur enregistrement a réussi.
    pub close_after_save: HashSet<PathBuf>,
    /// Fichiers ouverts récemment, le plus récent en premier.
    pub recent: Vec<PathBuf>,
    /// Menu contextuel demandé à cette image (clic droit sur un onglet).
    pub menu_request: Option<MenuRequest>,
    /// Clic droit dans le texte de l'éditeur actif (le menu est ouvert par l'application).
    pub editor_click: Option<EditorClick>,
    /// Onglet en cours de glisser.
    drag: Option<splits::TabDrag>,
}

/// Un clic droit dans l'éditeur : où, et s'il est tombé dans la sélection (qu'on garde).
#[derive(Debug, Clone, Copy)]
pub struct EditorClick {
    pub pos: egui::Pos2,
    pub inside_selection: bool,
}

impl Editors {
    fn fresh_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    // ----- Documents -------------------------------------------------------------

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.items.iter().position(|e| e.path() == Some(path))
    }

    fn doc_index(&self, doc: u64) -> Option<usize> {
        self.items.iter().position(|e| e.id == doc)
    }

    /// Un fichier ou un dossier a changé de chemin : les onglets le suivent, contenu non
    /// enregistré compris (l'onglet modifié ne réclame pas « changé sur le disque »).
    pub fn rename_path(&mut self, from: &Path, to: &Path) {
        for editor in &mut self.items {
            let Some(current) = editor.path().map(Path::to_path_buf) else {
                continue;
            };
            let new = if current == from {
                Some(to.to_path_buf())
            } else {
                current.strip_prefix(from).ok().map(|rest| to.join(rest))
            };
            if let Some(new) = new {
                editor.document.set_path(new);
                if editor.document.is_modified() {
                    editor.keep_mine = true;
                }
            }
        }
        for path in &mut self.recent {
            if let Ok(rest) = path.strip_prefix(from) {
                *path = to.join(rest);
            }
        }
        for group in &mut self.groups {
            group.reveal_active = true;
        }
    }

    fn touch_recent(&mut self, path: &Path) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(50);
    }

    // ----- Groupes et onglet actif ---------------------------------------------------

    pub fn group_index(&self, id: GroupId) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    /// Le groupe actif (le crée s'il n'y en a aucun).
    fn ensure_group(&mut self) -> GroupId {
        if self.group_index(self.active_group).is_some() {
            return self.active_group;
        }
        if let Some(first) = self.groups.first() {
            self.active_group = first.id;
            return self.active_group;
        }
        let id = self.fresh_id();
        self.groups.push(Group::new(id));
        self.root = Some(Node::Leaf(id));
        self.active_group = id;
        id
    }

    /// Les groupes dans l'ordre d'affichage (gauche à droite, haut en bas).
    pub fn group_order(&self) -> Vec<GroupId> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            root.leaves(&mut out);
        }
        out
    }

    pub fn active_group_ref(&self) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == self.active_group)
    }

    pub fn active_tab(&self) -> Option<&TabView> {
        let group = self.active_group_ref()?;
        group.tabs.get(group.active)
    }

    /// Le document de l'onglet actif du groupe actif.
    pub fn active_editor(&self) -> Option<&OpenEditor> {
        let tab = self.active_tab()?;
        self.items.iter().find(|e| e.id == tab.doc)
    }

    pub fn active_view(&self) -> Option<&EditorView> {
        self.active_tab().map(|t| &t.view)
    }

    pub fn active_view_mut(&mut self) -> Option<&mut EditorView> {
        let group = self.groups.iter_mut().find(|g| g.id == self.active_group)?;
        group.tabs.get_mut(group.active).map(|t| &mut t.view)
    }

    /// Le document et sa vue active, ensemble (menu contextuel, collage).
    pub fn active_pair_mut(&mut self) -> Option<(&mut OpenEditor, &mut EditorView)> {
        let group = self.groups.iter_mut().find(|g| g.id == self.active_group)?;
        let tab = group.tabs.get_mut(group.active)?;
        let doc = self.items.iter_mut().find(|e| e.id == tab.doc)?;
        Some((doc, &mut tab.view))
    }

    /// Nombre d'onglets du groupe actif.
    pub fn active_tab_count(&self) -> usize {
        self.active_group_ref().map_or(0, |g| g.tabs.len())
    }

    /// Nombre de vues de ce document, tous groupes confondus.
    pub fn view_count(&self, doc: u64) -> usize {
        self.groups
            .iter()
            .map(|g| g.tabs.iter().filter(|t| t.doc == doc).count())
            .sum()
    }

    /// Le document d'un onglet et le fait que cet onglet en soit la dernière vue.
    pub fn tab_info(&self, group: GroupId, index: usize) -> Option<(usize, bool)> {
        let g = self.groups.iter().find(|g| g.id == group)?;
        let tab = g.tabs.get(index)?;
        Some((self.doc_index(tab.doc)?, self.view_count(tab.doc) <= 1))
    }

    // ----- Ouvrir et activer ---------------------------------------------------------

    /// Ouvre (ou active) un fichier dans le **groupe actif**. Le chargement se fait en
    /// tâche de fond ; le saut a lieu à l'arrivée du contenu.
    pub fn open(&mut self, tasks: &Tasks, path: PathBuf, goto: Option<Goto>) {
        if let Some(index) = self.index_of(&path) {
            let doc = self.items[index].id;
            self.reveal_doc(doc);
            if let Some(goto) = goto {
                self.apply_goto_active(goto);
            }
            return;
        }
        if let Some(slot) = self.pending.get_mut(&path) {
            if goto.is_some() {
                slot.goto = goto;
            }
            return;
        }
        let groups = if self.group_index(self.active_group).is_some() {
            vec![self.active_group]
        } else {
            Vec::new()
        };
        self.pending.insert(
            path.clone(),
            Pending {
                goto,
                groups,
                restore: false,
            },
        );
        julie_text::spawn_load(tasks, path);
    }

    /// Montre un document déjà ouvert : l'onglet du groupe actif s'il y en a un, sinon
    /// celui d'un autre groupe (le groupe devient actif), sinon un nouvel onglet.
    fn reveal_doc(&mut self, doc: u64) {
        let active = self.ensure_group();
        let find = |g: &Group| g.tabs.iter().position(|t| t.doc == doc);
        if let Some(i) = self.group_index(active).and_then(|g| find(&self.groups[g])) {
            self.activate_tab(active, i);
            return;
        }
        let other = self.group_order().into_iter().find_map(|gid| {
            let g = self.group_index(gid)?;
            find(&self.groups[g]).map(|i| (gid, i))
        });
        if let Some((gid, i)) = other {
            self.activate_tab(gid, i);
            return;
        }
        if let Some(i) = self.add_tab(active, doc, false) {
            self.activate_tab(active, i);
        }
    }

    /// Active un onglet d'un groupe, qui devient le groupe actif.
    pub fn activate_tab(&mut self, group: GroupId, index: usize) {
        let Some(g) = self.group_index(group) else {
            return;
        };
        if index >= self.groups[g].tabs.len() {
            return;
        }
        let doc = self.groups[g].tabs[index].doc;
        if index != self.groups[g].active {
            let old = self.groups[g]
                .tabs
                .get(self.groups[g].active)
                .map(|t| t.doc);
            self.groups[g].previous = old;
        }
        self.groups[g].active = index;
        self.groups[g].reveal_active = true;
        self.groups[g].tabs[index].view.request_focus();
        self.active_group = group;
        if let Some(path) = self
            .doc_index(doc)
            .and_then(|d| self.items[d].path().map(Path::to_path_buf))
        {
            self.touch_recent(&path);
        }
    }

    /// Rend actif un groupe (clic dedans, Ctrl+Alt+Page) : le focus va à son éditeur.
    pub fn activate_group(&mut self, group: GroupId) {
        let Some(g) = self.group_index(group) else {
            return;
        };
        self.active_group = group;
        let active = self.groups[g].active;
        if let Some(tab) = self.groups[g].tabs.get_mut(active) {
            tab.view.request_focus();
        }
    }

    /// Groupe suivant (`1`) ou précédent (`-1`) dans l'ordre d'affichage.
    pub fn activate_group_relative(&mut self, delta: isize) {
        let order = self.group_order();
        if order.len() < 2 {
            return;
        }
        let at = order
            .iter()
            .position(|g| *g == self.active_group)
            .unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(order.len() as isize) as usize;
        self.activate_group(order[next]);
    }

    pub fn activate_relative(&mut self, delta: isize) {
        let Some(group) = self.active_group_ref() else {
            return;
        };
        let n = group.tabs.len() as isize;
        if n > 1 {
            let next = (group.active as isize + delta).rem_euclid(n);
            self.activate_tab(self.active_group, next as usize);
        }
    }

    /// Ctrl+Tab : revient à l'onglet précédemment actif du groupe.
    pub fn activate_previous(&mut self) {
        let Some(group) = self.active_group_ref() else {
            return;
        };
        let Some(previous) = group.previous else {
            return;
        };
        if let Some(i) = group.tabs.iter().position(|t| t.doc == previous) {
            self.activate_tab(self.active_group, i);
        }
    }

    /// Saute dans la vue active (après une ouverture : c'est celle du fichier).
    pub fn apply_goto_active(&mut self, goto: Goto) {
        self.sync_ownership();
        let Some((editor, view)) = self.active_pair_mut() else {
            return;
        };
        let buffer = editor.document.buffer();
        let start = buffer.position_to_offset(goto.position);
        let end = goto.end.map(|e| buffer.position_to_offset(e));
        editor.document.set_cursor(start, false);
        if let Some(end) = end {
            editor.document.set_cursor(end, true);
        }
        view.center_line(goto.position.line);
        view.reveal_cursor();
        view.request_focus();
    }

    // ----- Vues ----------------------------------------------------------------------

    fn make_view(&mut self, doc: u64, parked: Option<Parked>) -> TabView {
        let path = self
            .doc_index(doc)
            .and_then(|d| self.items[d].path().map(Path::to_path_buf))
            .unwrap_or_default();
        let vid = self.fresh_id();
        let mut view = EditorView::new((path, vid));
        view.theme = Some(theme::editor_theme(self.dark));
        TabView {
            doc,
            vid,
            view,
            parked,
        }
    }

    /// Ajoute une vue du document au groupe ; rend son indice. Si le groupe a déjà une
    /// vue de ce document, rend celle-là.
    fn add_tab(&mut self, group: GroupId, doc: u64, by_hint: bool) -> Option<usize> {
        let g = self.group_index(group)?;
        if let Some(i) = self.groups[g].tabs.iter().position(|t| t.doc == doc) {
            return Some(i);
        }
        let tab = self.make_view(doc, None);
        Some(self.insert_tab(g, tab, by_hint))
    }

    fn insert_tab(&mut self, g: usize, tab: TabView, by_hint: bool) -> usize {
        let mut at = self.groups[g].tabs.len();
        if by_hint {
            let hint = &self.groups[g].hint_order;
            let rank_of = |doc: u64| {
                let path = self
                    .items
                    .iter()
                    .find(|e| e.id == doc)
                    .and_then(|e| e.path());
                path.and_then(|p| hint.iter().position(|h| h == p))
            };
            if let Some(rank) = rank_of(tab.doc) {
                at = self.groups[g]
                    .tabs
                    .iter()
                    .position(|other| rank_of(other.doc).is_none_or(|r| r > rank))
                    .unwrap_or(at);
            }
        }
        let group = &mut self.groups[g];
        group.tabs.insert(at, tab);
        if at <= group.active && group.tabs.len() > 1 {
            group.active += 1;
        }
        at
    }

    /// Contenu arrivé : nouvel onglet, ou rechargement d'un document existant.
    pub fn on_loaded(&mut self, loaded: LoadedFile, waker: &julie_text::Waker, dark: bool) {
        self.dark = dark;
        let path = loaded.path.clone();
        let pending = self.pending.remove(&path);
        let goto = pending.as_ref().and_then(|p| p.goto);
        if let Some(index) = self.index_of(&path) {
            let editor = &mut self.items[index];
            editor.document.reload(loaded);
            editor.notice = None;
            editor.keep_mine = false;
            editor.confirm_reload = false;
            for group in &mut self.groups {
                for tab in &mut group.tabs {
                    tab.parked = None;
                }
            }
            if let Some(goto) = goto {
                let doc = self.items[index].id;
                self.reveal_doc(doc);
                self.apply_goto_active(goto);
            }
        } else {
            let mut document = Document::from_loaded(loaded);
            document.enable_syntax(Some(waker.clone()));
            let doc = self.fresh_id();
            self.items.push(OpenEditor {
                id: doc,
                document,
                notice: None,
                keep_mine: false,
                confirm_reload: false,
                owner: None,
            });
            let restore = pending.as_ref().is_some_and(|p| p.restore);
            let mut targets: Vec<GroupId> = pending
                .map(|p| p.groups)
                .unwrap_or_default()
                .into_iter()
                .filter(|g| self.group_index(*g).is_some())
                .collect();
            if targets.is_empty() {
                targets.push(self.ensure_group());
            }
            for group in targets {
                if restore {
                    self.add_tab(group, doc, true);
                } else if let Some(i) = self.add_tab(group, doc, false) {
                    self.activate_tab(group, i);
                }
            }
            if let Some(goto) = goto {
                self.apply_goto_active(goto);
            }
        }
        self.finish_restore_if_done();
    }

    /// Aucun chargement en cours.
    pub fn is_loaded(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn on_load_failed(&mut self, path: &Path) {
        self.pending.remove(path);
        self.finish_restore_if_done();
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        for group in &mut self.groups {
            for tab in &mut group.tabs {
                tab.view.theme = Some(theme::editor_theme(dark));
            }
        }
    }

    // ----- Curseur de chaque vue -------------------------------------------------------

    fn tab_mut_by_vid(&mut self, vid: u64) -> Option<&mut TabView> {
        self.groups
            .iter_mut()
            .flat_map(|g| g.tabs.iter_mut())
            .find(|t| t.vid == vid)
    }

    /// Le document de la vue active tient la sélection vivante ; celle qui la tenait
    /// range la sienne. À appeler avant d'afficher ou de lire un curseur.
    pub fn sync_ownership(&mut self) {
        let Some(tab) = self.active_tab() else {
            return;
        };
        let (doc_id, vid) = (tab.doc, tab.vid);
        let Some(d) = self.doc_index(doc_id) else {
            return;
        };
        let previous_owner = self.items[d].owner;
        if previous_owner == Some(vid) {
            return;
        }
        let parked_now = {
            let document = &self.items[d].document;
            Parked {
                selections: document.selections().clone(),
                version: document.buffer().version(),
            }
        };
        if let Some(old) = previous_owner {
            if let Some(old_tab) = self.tab_mut_by_vid(old) {
                old_tab.parked = Some(parked_now);
            }
        }
        let wanted = self.tab_mut_by_vid(vid).and_then(|t| t.parked.take());
        if let Some(parked) = wanted {
            let mapped = mapped_selections(&self.items[d].document, &parked);
            self.items[d].document.set_selections(mapped);
        }
        self.items[d].owner = Some(vid);
    }

    // ----- Fermer ----------------------------------------------------------------------

    /// Ferme un document : toutes ses vues disparaissent.
    pub fn close(&mut self, index: usize) {
        let Some(doc) = self.items.get(index).map(|e| e.id) else {
            return;
        };
        let emptied: Vec<GroupId> = self
            .groups
            .iter_mut()
            .filter_map(|group| {
                let before = group.tabs.len();
                let active_doc = group.tabs.get(group.active).map(|t| t.doc);
                let closed_at = group.tabs.iter().position(|t| t.doc == doc);
                group.tabs.retain(|t| t.doc != doc);
                if group.tabs.len() == before {
                    return None;
                }
                if group.previous == Some(doc) {
                    group.previous = None;
                }
                // Garde le même onglet actif ; si c'était celui fermé, le voisin.
                group.active = match (active_doc, closed_at) {
                    (Some(a), _) if a != doc => group
                        .tabs
                        .iter()
                        .position(|t| t.doc == a)
                        .unwrap_or_default(),
                    _ => group.active.min(group.tabs.len().saturating_sub(1)),
                };
                group.reveal_active = true;
                group.tabs.is_empty().then_some(group.id)
            })
            .collect();
        self.items.remove(index);
        for group in emptied {
            self.remove_group(group);
        }
        self.focus_active();
    }

    /// Ferme une seule vue ; le document disparaît avec sa dernière vue.
    pub fn close_view(&mut self, group: GroupId, index: usize) {
        let Some(g) = self.group_index(group) else {
            return;
        };
        if index >= self.groups[g].tabs.len() {
            return;
        }
        let doc = self.groups[g].tabs[index].doc;
        if self.view_count(doc) <= 1 {
            if let Some(d) = self.doc_index(doc) {
                self.close(d);
            }
            return;
        }
        self.take_tab(group, index);
        self.drop_group_if_empty(group);
        self.focus_active();
    }

    /// Retire un onglet d'un groupe (sans fermer le document) et le rend.
    fn take_tab(&mut self, group: GroupId, index: usize) -> Option<TabView> {
        let g = self.group_index(group)?;
        let grp = &mut self.groups[g];
        if index >= grp.tabs.len() {
            return None;
        }
        let tab = grp.tabs.remove(index);
        if index < grp.active {
            grp.active -= 1;
        } else if grp.active >= grp.tabs.len() {
            grp.active = grp.tabs.len().saturating_sub(1);
        }
        grp.reveal_active = true;
        Some(tab)
    }

    fn drop_group_if_empty(&mut self, group: GroupId) {
        if self
            .group_index(group)
            .is_some_and(|g| self.groups[g].tabs.is_empty())
        {
            self.remove_group(group);
        }
    }

    /// Supprime un groupe de l'arbre : son voisin reprend la place et devient actif.
    fn remove_group(&mut self, group: GroupId) {
        let neighbor = self.root.as_ref().and_then(|r| r.neighbor(group));
        self.root = self.root.take().and_then(|r| r.remove(group));
        self.groups.retain(|g| g.id != group);
        if self.active_group == group || self.group_index(self.active_group).is_none() {
            self.active_group = neighbor
                .filter(|n| self.group_index(*n).is_some())
                .or_else(|| self.group_order().first().copied())
                .unwrap_or(0);
        }
    }

    fn focus_active(&mut self) {
        if let Some(view) = self.active_view_mut() {
            view.request_focus();
        }
    }

    /// Ferme la vue de ce fichier dans le groupe actif (sans question : l'appelant
    /// a vérifié qu'il n'y a rien à enregistrer). `false` : pas d'onglet à fermer.
    pub fn close_path_in_active_group(&mut self, path: &Path) -> bool {
        let group = self.active_group;
        let Some(g) = self.group_index(group) else {
            return false;
        };
        let at = self.groups[g].tabs.iter().position(|t| {
            self.items
                .iter()
                .find(|e| e.id == t.doc)
                .and_then(|e| e.path())
                == Some(path)
        });
        match at {
            Some(i) => {
                self.close_view(group, i);
                true
            }
            None => false,
        }
    }

    /// Les chemins des onglets du groupe actif, dans l'ordre.
    pub fn active_group_paths(&self) -> Vec<PathBuf> {
        self.active_group_ref()
            .map(|g| {
                g.tabs
                    .iter()
                    .filter_map(|t| {
                        let d = self.doc_index(t.doc)?;
                        self.items[d].path().map(Path::to_path_buf)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    // ----- Enregistrer, disque ---------------------------------------------------------

    pub fn modified_paths(&self) -> Vec<PathBuf> {
        self.items
            .iter()
            .filter(|e| e.document.is_modified())
            .filter_map(|e| e.path().map(Path::to_path_buf))
            .collect()
    }

    pub fn any_modified(&self) -> bool {
        self.items.iter().any(|e| e.document.is_modified())
    }

    /// Enregistre tous les documents modifiés (une seule fois chacun, même ouverts
    /// dans deux groupes). Rend les erreurs de préparation (encodage…) ; les résultats
    /// d'écriture arrivent en `TextEvent`.
    pub fn save_all(&mut self, tasks: &Tasks) -> Vec<String> {
        let mut errors = Vec::new();
        for editor in &mut self.items {
            if editor.document.is_modified() {
                match editor.document.save_request() {
                    Ok(request) => {
                        julie_text::spawn_save(tasks, request);
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        errors
    }

    pub fn save_paths(&mut self, tasks: &Tasks, paths: &[PathBuf]) -> Vec<String> {
        let mut errors = Vec::new();
        for editor in &mut self.items {
            let wanted = editor.path().is_some_and(|p| paths.iter().any(|q| q == p));
            if wanted && editor.document.is_modified() {
                match editor.document.save_request() {
                    Ok(request) => {
                        julie_text::spawn_save(tasks, request);
                    }
                    Err(error) => errors.push(error.to_string()),
                }
            }
        }
        errors
    }

    pub fn on_saved(&mut self, outcome: &SaveOutcome) {
        if let Some(index) = self.index_of(&outcome.path) {
            let editor = &mut self.items[index];
            editor.document.mark_saved(outcome);
            editor.notice = None;
            editor.keep_mine = false;
        }
        if self.close_after_save.remove(&outcome.path) {
            if let Some(index) = self.index_of(&outcome.path) {
                self.close(index);
            }
        }
    }

    pub fn check_disk(&self, tasks: &Tasks) {
        let files: Vec<_> = self
            .items
            .iter()
            .filter_map(|e| Some((e.path()?.to_path_buf(), e.document.disk_stamp())))
            .collect();
        if !files.is_empty() {
            julie_text::spawn_check_disk(tasks, files);
        }
    }

    pub fn on_disk_checked(&mut self, states: Vec<(PathBuf, DiskState)>, tasks: &Tasks) {
        for (path, state) in states {
            let Some(index) = self.index_of(&path) else {
                continue;
            };
            let editor = &mut self.items[index];
            match state {
                DiskState::Unchanged => {}
                DiskState::Changed if !editor.document.is_modified() => {
                    julie_text::spawn_load(tasks, path);
                }
                DiskState::Changed => {
                    if !editor.keep_mine {
                        editor.notice = Some(Notice::Changed);
                    }
                }
                DiskState::Deleted => {
                    editor.document.mark_deleted_on_disk();
                    editor.notice = Some(Notice::Deleted);
                }
            }
        }
    }

    /// Contenu en mémoire des onglets modifiés : cherché à la place du disque.
    pub fn unsaved_files(&self) -> UnsavedFiles {
        self.items
            .iter()
            .filter(|e| e.document.is_modified())
            .filter_map(|e| {
                Some((
                    e.path()?.to_path_buf(),
                    Arc::<str>::from(e.document.text().as_str()),
                ))
            })
            .collect()
    }

    pub fn unsaved_text(&self, path: &Path) -> Option<Arc<str>> {
        let editor = self.items.get(self.index_of(path)?)?;
        editor
            .document
            .is_modified()
            .then(|| Arc::<str>::from(editor.document.text().as_str()))
    }

    /// Titres des onglets d'un groupe : le nom, plus le dossier parent si deux onglets
    /// du groupe ont le même nom.
    pub fn titles(&self, group: GroupId) -> Vec<String> {
        let Some(group) = self.groups.iter().find(|g| g.id == group) else {
            return Vec::new();
        };
        let docs: Vec<&OpenEditor> = group
            .tabs
            .iter()
            .filter_map(|t| self.items.iter().find(|e| e.id == t.doc))
            .collect();
        let names: Vec<String> = docs
            .iter()
            .map(|e| {
                e.path()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| t::UNTITLED.to_owned())
            })
            .collect();
        names
            .iter()
            .zip(&docs)
            .map(|(name, editor)| {
                let duplicate = names.iter().filter(|n| *n == name).count() > 1;
                let parent = editor
                    .path()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned());
                match (duplicate, parent) {
                    (true, Some(parent)) => format!("{name} — {parent}"),
                    _ => name.clone(),
                }
            })
            .collect()
    }

    // ----- Fractionner ---------------------------------------------------------------

    /// « Fractionner » : un nouveau groupe à côté du groupe actif, avec une deuxième vue
    /// du même document (`copy`), ou, sinon, l'onglet déplacé (`copy = false`). `false`
    /// si rien n'a changé.
    pub fn split_active(&mut self, dir: Dir, copy: bool) -> bool {
        let source = self.active_group;
        let Some(g) = self.group_index(source) else {
            return false;
        };
        let group = &self.groups[g];
        if group.tabs.get(group.active).is_none() || (!copy && group.tabs.len() < 2) {
            return false;
        }
        self.sync_ownership();
        let new_id = self.fresh_id();
        let mut new_group = Group::new(new_id);
        if copy {
            let (doc, scroll) = {
                let tab = &self.groups[g].tabs[self.groups[g].active];
                (tab.doc, tab.view.scroll_y())
            };
            let parked = self.doc_index(doc).map(|d| {
                let document = &self.items[d].document;
                Parked {
                    selections: document.selections().clone(),
                    version: document.buffer().version(),
                }
            });
            let mut tab = self.make_view(doc, parked);
            tab.view.set_scroll_y(scroll);
            new_group.tabs.push(tab);
        } else {
            let active = self.groups[g].active;
            let Some(tab) = self.take_tab(source, active) else {
                return false;
            };
            new_group.tabs.push(tab);
        }
        self.groups.push(new_group);
        if let Some(root) = &mut self.root {
            root.split_leaf(source, new_id, dir);
        }
        self.activate_tab(new_id, 0);
        true
    }

    /// « Déplacer vers l'autre groupe » : l'onglet actif passe au groupe suivant.
    pub fn move_to_other_group(&mut self) -> bool {
        let order = self.group_order();
        if order.len() < 2 {
            return false;
        }
        let source = self.active_group;
        let at = order.iter().position(|g| *g == source).unwrap_or(0);
        let target = order[(at + 1) % order.len()];
        let Some(s) = self.group_index(source) else {
            return false;
        };
        let index = self.groups[s].active;
        self.move_tab(source, index, target, None)
    }

    /// Déplace un onglet dans un groupe, à `at` (fin si `None`). Si le groupe montre déjà ce
    /// document, c'est cet onglet qui est activé et la vue déplacée disparaît. Le groupe
    /// de départ, vidé, est supprimé.
    pub fn move_tab(
        &mut self,
        from: GroupId,
        index: usize,
        to: GroupId,
        at: Option<usize>,
    ) -> bool {
        if self.group_index(to).is_none() {
            return false;
        }
        self.sync_ownership();
        let Some(tab) = self.take_tab(from, index) else {
            return false;
        };
        let doc = tab.doc;
        let Some(t) = self.group_index(to) else {
            return false;
        };
        if let Some(existing) = self.groups[t].tabs.iter().position(|x| x.doc == doc) {
            // Déjà montré là : on garde cette vue-là.
            if let Some(d) = self.doc_index(doc) {
                if self.items[d].owner == Some(tab.vid) {
                    self.items[d].owner = None;
                }
            }
            self.drop_group_if_empty(from);
            self.activate_tab(to, existing);
            return true;
        }
        let same_group = from == to;
        let mut position = at.unwrap_or(self.groups[t].tabs.len());
        if same_group && at.is_some_and(|a| a > index) {
            position = position.saturating_sub(1);
        }
        let position = position.min(self.groups[t].tabs.len());
        self.groups[t].tabs.insert(position, tab);
        if !same_group && position <= self.groups[t].active && self.groups[t].tabs.len() > 1 {
            self.groups[t].active += 1;
        }
        self.drop_group_if_empty(from);
        self.activate_tab(to, position);
        true
    }

    /// Glisser sur le bord d'un groupe : un nouveau groupe de ce côté reçoit l'onglet.
    pub fn move_tab_to_edge(
        &mut self,
        from: GroupId,
        index: usize,
        target: GroupId,
        edge: Edge,
    ) -> bool {
        let Some(f) = self.group_index(from) else {
            return false;
        };
        if self.group_index(target).is_none() {
            return false;
        }
        // Un groupe d'un seul onglet n'a rien à fractionner avec lui-même.
        if from == target && self.groups[f].tabs.len() < 2 {
            return false;
        }
        self.sync_ownership();
        let Some(tab) = self.take_tab(from, index) else {
            return false;
        };
        let new_id = self.fresh_id();
        let mut group = Group::new(new_id);
        group.tabs.push(tab);
        self.groups.push(group);
        if let Some(root) = &mut self.root {
            root.insert_beside(target, new_id, edge);
        }
        self.drop_group_if_empty(from);
        self.activate_tab(new_id, 0);
        true
    }

    /// « Tout regrouper » : un seul groupe, chaque document une seule fois.
    pub fn merge_all(&mut self) -> bool {
        let order = self.group_order();
        if order.len() < 2 {
            return false;
        }
        self.sync_ownership();
        let keep = order[0];
        let active_vid = self.active_tab().map(|t| t.vid);
        let active_doc = self.active_tab().map(|t| t.doc);
        let mut moved: Vec<TabView> = Vec::new();
        for gid in &order[1..] {
            if let Some(g) = self.group_index(*gid) {
                moved.append(&mut self.groups[g].tabs);
            }
        }
        self.groups.retain(|g| g.id == keep);
        self.root = Some(Node::Leaf(keep));
        for tab in moved {
            if self.groups[0].tabs.iter().any(|t| t.doc == tab.doc) {
                if let Some(d) = self.doc_index(tab.doc) {
                    if self.items[d].owner == Some(tab.vid) {
                        self.items[d].owner = None;
                    }
                }
                continue;
            }
            self.groups[0].tabs.push(tab);
        }
        self.active_group = keep;
        let at = self.groups[0]
            .tabs
            .iter()
            .position(|t| Some(t.vid) == active_vid)
            .or_else(|| {
                self.groups[0]
                    .tabs
                    .iter()
                    .position(|t| Some(t.doc) == active_doc)
            })
            .unwrap_or(0);
        self.activate_tab(keep, at);
        true
    }

    // ----- Restauration ----------------------------------------------------------------

    /// Quand tout est chargé : retire les groupes restés vides (fichier disparu), fixe
    /// l'onglet actif de chacun et le groupe actif.
    fn finish_restore_if_done(&mut self) {
        if !self.pending.is_empty() {
            return;
        }
        let Some(restoring) = self.restoring.take() else {
            return;
        };
        let order = self.group_order();
        let empty: Vec<GroupId> = self
            .groups
            .iter()
            .filter(|g| g.tabs.is_empty())
            .map(|g| g.id)
            .collect();
        let wanted = order.get(restoring.active_leaf).copied();
        for gid in empty {
            self.remove_group(gid);
        }
        for g in 0..self.groups.len() {
            let hint = self.groups[g].hint_active.take();
            self.groups[g].hint_order.clear();
            if let Some(hint) = hint {
                let at = self.groups[g].tabs.iter().position(|t| {
                    self.items
                        .iter()
                        .find(|e| e.id == t.doc)
                        .and_then(|e| e.path())
                        == Some(hint.as_path())
                });
                if let Some(at) = at {
                    self.groups[g].active = at;
                }
            }
            self.groups[g].reveal_active = true;
        }
        let target = wanted
            .filter(|w| self.group_index(*w).is_some())
            .or_else(|| self.group_order().first().copied());
        if let Some(target) = target {
            self.activate_group(target);
        }
    }

    /// Une restauration est en cours (les fichiers arrivent).
    pub fn is_restoring(&self) -> bool {
        self.restoring.is_some()
    }
}

/// Les sélections rangées d'une vue, recalées sur les modifications faites depuis.
fn mapped_selections(document: &Document, parked: &Parked) -> Selections {
    let buffer = document.buffer();
    let map = |offset: usize| {
        buffer
            .map_offset_since(offset, parked.version, Bias::After)
            .unwrap_or(offset)
            .min(buffer.len_chars())
    };
    parked.selections.map(|s| Selection {
        anchor: map(s.anchor),
        head: map(s.head),
        goal_column: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un document en mémoire, ouvert comme le ferait `open` dans le groupe actif.
    fn add(editors: &mut Editors, name: &str, text: &str) -> u64 {
        let id = editors.fresh_id();
        let mut document = Document::new(text);
        document.set_path(PathBuf::from(format!("/p/{name}")));
        editors.items.push(OpenEditor {
            id,
            document,
            notice: None,
            keep_mine: false,
            confirm_reload: false,
            owner: None,
        });
        let group = editors.ensure_group();
        let at = editors.add_tab(group, id, false).unwrap();
        editors.activate_tab(group, at);
        id
    }

    fn names(editors: &Editors, group: GroupId) -> Vec<String> {
        editors.titles(group)
    }

    #[test]
    fn split_gives_two_views_of_one_document() {
        let mut e = Editors::default();
        let a = add(&mut e, "a.php", "hello");
        assert!(e.split_active(Dir::Vertical, true));
        assert_eq!(e.groups.len(), 2);
        assert_eq!(e.items.len(), 1);
        assert_eq!(e.view_count(a), 2);
        assert_eq!(e.group_order().len(), 2);
        // Une frappe : un seul document, donc une seule marque.
        e.sync_ownership();
        let editor = e.active_pair_mut().unwrap().0;
        editor.document.insert_text("X");
        assert!(editor.document.is_modified());
        assert_eq!(e.modified_paths().len(), 1);
    }

    #[test]
    fn closing_one_view_keeps_the_document() {
        let mut e = Editors::default();
        let a = add(&mut e, "a.php", "hello");
        e.split_active(Dir::Horizontal, true);
        let group = e.active_group;
        e.close_view(group, 0);
        assert_eq!(e.items.len(), 1, "la dernière vue est encore là");
        assert_eq!(e.groups.len(), 1, "le groupe vidé disparaît");
        assert_eq!(e.view_count(a), 1);
        assert!(matches!(e.root, Some(Node::Leaf(_))));
        let group = e.active_group;
        e.close_view(group, 0);
        assert!(e.items.is_empty());
        assert!(e.groups.is_empty());
        assert!(e.root.is_none());
    }

    #[test]
    fn move_needs_a_second_tab_and_leaves_the_first_group() {
        let mut e = Editors::default();
        add(&mut e, "a.php", "a");
        assert!(!e.split_active(Dir::Horizontal, false), "un seul onglet");
        add(&mut e, "b.js", "b");
        assert!(e.split_active(Dir::Horizontal, false));
        let order = e.group_order();
        assert_eq!(names(&e, order[0]), ["a.php"]);
        assert_eq!(names(&e, order[1]), ["b.js"]);
        assert_eq!(e.active_group, order[1]);
    }

    #[test]
    fn closing_the_last_tab_of_a_group_gives_its_room_to_the_neighbour() {
        let mut e = Editors::default();
        add(&mut e, "a.php", "a");
        add(&mut e, "b.js", "b");
        e.split_active(Dir::Vertical, false);
        e.split_active(Dir::Horizontal, true); // la droite coupée en deux
        assert_eq!(e.groups.len(), 3);
        assert_eq!(e.root.as_ref().unwrap().depth(), 3);
        let bottom = e.active_group;
        e.close_view(bottom, 0);
        assert_eq!(e.groups.len(), 2);
        assert_eq!(e.root.as_ref().unwrap().depth(), 2);
        assert_eq!(e.items.len(), 2, "aucun fichier perdu");
    }

    #[test]
    fn merge_all_keeps_each_file_once() {
        let mut e = Editors::default();
        add(&mut e, "a.php", "a");
        add(&mut e, "b.js", "b");
        e.split_active(Dir::Vertical, true); // b.js des deux côtés
        e.split_active(Dir::Horizontal, true); // et encore une copie
        assert!(e.groups.len() >= 2);
        assert!(e.merge_all());
        assert_eq!(e.groups.len(), 1);
        let group = e.group_order()[0];
        let mut titles = names(&e, group);
        titles.sort();
        assert_eq!(titles, ["a.php", "b.js"]);
        assert_eq!(e.items.len(), 2);
        assert!(matches!(e.root, Some(Node::Leaf(_))));
    }

    #[test]
    fn moving_a_tab_onto_a_group_that_shows_it_does_not_duplicate() {
        let mut e = Editors::default();
        add(&mut e, "a.php", "a");
        add(&mut e, "b.js", "b");
        e.split_active(Dir::Vertical, true); // b.js à gauche et à droite
        let order = e.group_order();
        let right = order[1];
        // Le b.js de droite va vers la gauche, qui l'a déjà : une seule vue reste.
        assert!(e.move_tab(right, 0, order[0], None));
        assert_eq!(e.groups.len(), 1);
        assert_eq!(names(&e, order[0]), ["a.php", "b.js"]);
    }

    #[test]
    fn each_view_keeps_its_own_cursor() {
        let mut e = Editors::default();
        let a = add(&mut e, "a.php", "0123456789\nabcdefghij\n");
        e.sync_ownership();
        e.items[0].document.set_cursor(2, false);
        e.split_active(Dir::Vertical, true);
        e.sync_ownership();
        // Dans la nouvelle vue : curseur ailleurs.
        e.items[0].document.set_cursor(15, false);
        let right = e.active_group;
        let left = e.group_order()[0];
        // On revient à gauche : son curseur d'origine.
        e.activate_group(left);
        e.sync_ownership();
        assert_eq!(e.items[0].document.selections().primary().head, 2);
        // Et à droite : le sien aussi.
        e.activate_group(right);
        e.sync_ownership();
        assert_eq!(e.items[0].document.selections().primary().head, 15);
        // Une insertion avant le curseur de l'autre vue le décale.
        e.items[0].document.set_cursor(0, false);
        e.items[0].document.insert_text("ZZ");
        e.activate_group(left);
        e.sync_ownership();
        assert_eq!(e.items[0].document.selections().primary().head, 4);
        assert_eq!(e.view_count(a), 2);
    }

    #[test]
    fn tree_operations() {
        let mut node = Node::Leaf(1);
        assert!(node.split_leaf(1, 2, Dir::Vertical));
        assert!(node.split_leaf(2, 3, Dir::Horizontal));
        assert!(node.insert_beside(1, 4, Edge::Top));
        let mut order = Vec::new();
        node.leaves(&mut order);
        assert_eq!(order, [4, 1, 2, 3]);
        assert_eq!(node.depth(), 3);
        assert_eq!(node.neighbor(3), Some(2));
        assert_eq!(node.neighbor(2), Some(3));
        let node = node.remove(2).unwrap();
        let mut order = Vec::new();
        node.leaves(&mut order);
        assert_eq!(order, [4, 1, 3]);
        assert!(Node::Leaf(9).remove(9).is_none());
    }

    #[test]
    fn layout_survives_a_json_round_trip() {
        let mut e = Editors::default();
        add(&mut e, "a.php", "a");
        add(&mut e, "b.js", "b");
        e.split_active(Dir::Vertical, false);
        let root = Path::new("/p");
        let json = e.layout_json(root).unwrap();
        let text = json.to_string();
        assert!(text.contains("\"split\":\"vertical\""), "{text}");
        assert!(text.contains("a.php") && text.contains("b.js"), "{text}");
        assert!(!text.contains("/p/"), "chemins relatifs au projet : {text}");
    }
}
