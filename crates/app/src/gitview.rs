//! Fenêtre Git (onglet de la fenêtre d'outils du bas) : changements locaux avec
//! cases à cocher et zone de message, diff côte à côte du fichier choisi, journal
//! paginé, console des commandes, fichiers en conflit.
//!
//! Cette vue n'a aucune règle Git : elle montre ce que `julie-vcs` publie sur le
//! bus (`VcsEvent`) et renvoie des [`GitIntent`] que `app` exécute.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, EventFilter, FontId, Id, Key, Modifiers, Rect, RichText,
    Sense, TextEdit, Ui,
};
use julie_core::TaskId;
use julie_vcs::{
    Change, CommandRecord, CommandResult, CommitDetails, CommitFile, CommitRequest, CommitSummary,
    ConflictKind, ConflictSide, DiffRequest, FileKind, FileStatus, Group, LogPage, LogQuery,
    OperationKind, RefKind, Status, VcsError, VcsEvent,
};

use crate::diffpane::DiffPane;
use crate::icons::{self, Icon};
use crate::kit;
use crate::texts as t;
use crate::theme::Palette;

const ROW: f32 = 22.0;
const TAB_BAR: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitTab {
    Changes,
    Log,
    Console,
}

impl GitTab {
    pub const ALL: [GitTab; 3] = [GitTab::Changes, GitTab::Log, GitTab::Console];

    pub fn key(self) -> &'static str {
        match self {
            GitTab::Changes => "changes",
            GitTab::Log => "log",
            GitTab::Console => "console",
        }
    }

    pub fn parse(text: &str) -> Option<GitTab> {
        GitTab::ALL.into_iter().find(|tab| tab.key() == text)
    }

    fn label(self) -> &'static str {
        match self {
            GitTab::Changes => t::GIT_TAB_CHANGES,
            GitTab::Log => t::GIT_TAB_LOG,
            GitTab::Console => t::GIT_TAB_CONSOLE,
        }
    }
}

/// Où va un diff demandé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTarget {
    /// L'aperçu à droite de la liste des changements.
    Preview,
    /// La grande fenêtre de diff.
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffJob {
    pub target: DiffTarget,
    pub token: u64,
}

/// Ce que la vue demande à `app` de faire.
#[derive(Debug)]
pub enum GitIntent {
    Refresh,
    RequestDiff {
        request: DiffRequest,
        job: DiffJob,
    },
    OpenFile(PathBuf),
    Commit(CommitRequest),
    ContinueOperation(Option<String>),
    AbortOperation,
    AcceptSide {
        paths: Vec<PathBuf>,
        side: ConflictSide,
    },
    OpenMerge(PathBuf),
    LoadLog(LogQuery),
    LoadCommit(String),
    Notify(String),
}

/// Question posée avant une action qui écrase du travail.
#[derive(Debug, Clone)]
pub enum Confirm {
    KeepSide {
        side: ConflictSide,
        files: Vec<(PathBuf, bool)>,
    },
    AbortOperation(OperationKind),
}

/// Ce que l'application sait du dépôt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    Unknown,
    NoRepository,
    Ready,
}

#[derive(Default)]
struct LogState {
    commits: Vec<CommitSummary>,
    has_more: bool,
    loading: bool,
    requested: bool,
    selected: Option<usize>,
    details: Option<CommitDetails>,
    details_for: Option<String>,
    file_selected: Option<usize>,
}

pub struct GitView {
    pub tab: GitTab,
    status: Option<Arc<Status>>,
    checked: HashSet<PathBuf>,
    known: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
    pub message: String,
    pub amend: bool,
    focus_message: bool,
    commit_pending: bool,
    commit_error: Option<String>,
    merge_prefilled: bool,
    pub preview: DiffPane,
    preview_token: u64,
    preview_dirty: bool,
    jobs: HashMap<TaskId, DiffJob>,
    log: LogState,
    console: Vec<CommandRecord>,
    console_open: HashSet<u64>,
    pub confirm: Option<Confirm>,
    list_focus: Id,
    message_id: Id,
    /// Fichier dont la fusion se prépare (chargement des versions).
    pub merge_loading: Option<PathBuf>,
    /// Le journal prend le focus clavier à la prochaine image.
    log_focus_request: bool,
}

impl GitView {
    pub fn new(dark: bool) -> Self {
        Self {
            tab: GitTab::Changes,
            status: None,
            checked: HashSet::new(),
            known: HashSet::new(),
            selected: None,
            message: String::new(),
            amend: false,
            focus_message: false,
            commit_pending: false,
            commit_error: None,
            merge_prefilled: false,
            preview: DiffPane::new("git-apercu", dark),
            preview_token: 0,
            preview_dirty: false,
            jobs: HashMap::new(),
            log: LogState::default(),
            console: Vec::new(),
            console_open: HashSet::new(),
            confirm: None,
            list_focus: Id::unique("git-liste"),
            message_id: Id::unique("git-message"),
            merge_loading: None,
            log_focus_request: false,
        }
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.preview.set_dark(dark);
    }

    // ----- Événements ------------------------------------------------------------

    pub fn on_status(&mut self, status: &Status) {
        if self.status.as_deref() == Some(status) {
            return;
        }
        let present: HashSet<&PathBuf> = status.files.iter().map(|f| &f.path).collect();
        for file in &status.files {
            if self.known.insert(file.path.clone())
                && file.group() == Group::Changed
                && status.operation.is_none()
            {
                self.checked.insert(file.path.clone());
            }
        }
        self.known.retain(|p| present.contains(p));
        self.checked.retain(|p| present.contains(p));
        match &self.selected {
            Some(selected) if !present.contains(selected) => {
                self.selected = None;
                self.preview.clear();
            }
            Some(selected) => {
                let before = self.status.as_ref().and_then(|s| s.find(selected));
                if before != status.find(selected) {
                    self.preview_dirty = true;
                }
            }
            None => {}
        }
        match &status.operation {
            Some(operation) => {
                if status.conflicts().next().is_none()
                    && !self.merge_prefilled
                    && self.message.trim().is_empty()
                {
                    if let Some(text) = &operation.message {
                        self.message = clean_merge_message(text);
                        self.merge_prefilled = true;
                    }
                }
            }
            None => self.merge_prefilled = false,
        }
        self.status = Some(Arc::new(status.clone()));
    }

    /// À rappeler quand un fichier a changé sur le disque : l'aperçu se refait.
    pub fn invalidate_diff(&mut self) {
        if self.selected.is_some() {
            self.preview_dirty = true;
        }
    }

    pub fn note_diff_task(&mut self, task: TaskId, job: DiffJob) {
        self.jobs.insert(task, job);
    }

    pub fn take_job(&mut self, task: Option<TaskId>) -> Option<DiffJob> {
        self.jobs.remove(&task?)
    }

    /// Un diff calculé est arrivé : faut-il encore l'afficher ?
    pub fn accepts(&self, job: DiffJob) -> bool {
        job.target == DiffTarget::Window || job.token == self.preview_token
    }

    pub fn set_preview(&mut self, prepared: crate::diffpane::Prepared, waker: &julie_text::Waker) {
        self.preview.set(prepared, waker);
    }

    pub fn on_vcs(&mut self, event: &VcsEvent, out: &mut Vec<GitIntent>) {
        match event {
            VcsEvent::Command(record) => {
                match self.console.iter_mut().find(|r| r.id == record.id) {
                    Some(slot) => *slot = record.clone(),
                    None => self.console.push(record.clone()),
                }
                if self.console.len() > 500 {
                    self.console.remove(0);
                }
            }
            VcsEvent::Log(page) => self.on_log(page),
            VcsEvent::CommitDetails(details) => {
                if self.log.details_for.as_deref() == Some(details.summary.id.as_str()) {
                    self.log.details = Some(details.clone());
                    self.log.details_for = None;
                }
            }
            VcsEvent::Committed(outcome) => {
                self.commit_pending = false;
                self.commit_error = None;
                self.message.clear();
                self.amend = false;
                self.reset_log();
                out.push(GitIntent::Notify(t::commit_done(&outcome.subject)));
            }
            VcsEvent::OperationContinued(outcome) => {
                self.commit_pending = false;
                if outcome.finished {
                    self.message.clear();
                    self.merge_prefilled = false;
                    self.reset_log();
                    out.push(GitIntent::Notify(t::OPERATION_DONE.to_owned()));
                }
            }
            VcsEvent::OperationAborted(_) => {
                self.message.clear();
                self.merge_prefilled = false;
                self.reset_log();
            }
            VcsEvent::Failed(failure) => match failure.action {
                julie_vcs::Action::Commit | julie_vcs::Action::Continue => {
                    self.commit_pending = false;
                    self.commit_error = Some(failure_text(&failure.error));
                }
                julie_vcs::Action::Log => self.log.loading = false,
                julie_vcs::Action::Conflict => self.merge_loading = None,
                _ => {}
            },
            _ => {}
        }
    }

    fn reset_log(&mut self) {
        self.log.commits.clear();
        self.log.selected = None;
        self.log.details = None;
        self.log.requested = false;
        self.log.loading = false;
    }

    fn on_log(&mut self, page: &LogPage) {
        self.log.loading = false;
        if page.query.skip == 0 {
            self.log.commits.clear();
        }
        if page.query.skip == self.log.commits.len() {
            self.log.commits.extend(page.commits.iter().cloned());
            self.log.has_more = page.has_more;
        }
    }

    // ----- Commandes (raccourcis, scénario) --------------------------------------

    pub fn show_changes_for_commit(&mut self) {
        self.tab = GitTab::Changes;
        self.focus_message = true;
    }

    /// Sélectionne un fichier (chemin absolu ou relatif au dépôt).
    pub fn select_path(&mut self, wanted: &str) -> bool {
        let Some(status) = &self.status else {
            return false;
        };
        let wanted = wanted.replace('\\', "/");
        let found = status
            .files
            .iter()
            .find(|f| {
                f.repo_path == wanted || f.path.to_string_lossy().replace('\\', "/") == wanted
            })
            .map(|f| f.path.clone());
        match found {
            Some(path) => {
                self.select(path);
                true
            }
            None => false,
        }
    }

    fn select(&mut self, path: PathBuf) {
        if self.selected.as_ref() != Some(&path) {
            self.selected = Some(path);
            self.preview.clear();
            self.preview_dirty = true;
        }
    }

    pub fn set_checked(&mut self, wanted: &str, value: bool) -> bool {
        let Some(status) = &self.status else {
            return false;
        };
        let wanted = wanted.replace('\\', "/");
        let found = status
            .files
            .iter()
            .find(|f| f.repo_path == wanted)
            .map(|f| f.path.clone());
        match found {
            Some(path) => {
                if value {
                    self.checked.insert(path);
                } else {
                    self.checked.remove(&path);
                }
                true
            }
            None => false,
        }
    }

    pub fn status(&self) -> Option<&Arc<Status>> {
        self.status.as_ref()
    }

    /// L'intention de valider ce qui est coché (bouton, Ctrl+Entrée, scénario).
    pub fn commit_intent(&mut self) -> Option<GitIntent> {
        let status = self.status.clone()?;
        if self.commit_pending || self.message.trim().is_empty() {
            return None;
        }
        if status.operation.is_some() {
            if status.conflicts().next().is_some() {
                return None;
            }
            self.commit_pending = true;
            return Some(GitIntent::ContinueOperation(Some(self.message.clone())));
        }
        let paths: Vec<PathBuf> = status
            .files
            .iter()
            .filter(|f| f.group() != Group::Conflicted && self.checked.contains(&f.path))
            .map(|f| f.path.clone())
            .collect();
        if paths.is_empty() && !self.amend {
            return None;
        }
        self.commit_pending = true;
        self.commit_error = None;
        Some(GitIntent::Commit(CommitRequest {
            paths,
            message: self.message.clone(),
            amend: self.amend,
        }))
    }

    fn can_commit(&self) -> bool {
        let Some(status) = &self.status else {
            return false;
        };
        if self.commit_pending || self.message.trim().is_empty() {
            return false;
        }
        if status.operation.is_some() {
            return status.conflicts().next().is_none();
        }
        self.amend
            || status
                .files
                .iter()
                .any(|f| f.group() != Group::Conflicted && self.checked.contains(&f.path))
    }

    fn selected_file(&self) -> Option<&FileStatus> {
        let selected = self.selected.as_ref()?;
        self.status
            .as_ref()?
            .files
            .iter()
            .find(|f| &f.path == selected)
    }

    fn diff_intent(&mut self, file: &FileStatus, target: DiffTarget, out: &mut Vec<GitIntent>) {
        if file.group() == Group::Conflicted {
            return;
        }
        if target == DiffTarget::Preview {
            self.preview_token += 1;
        }
        let token = self.preview_token;
        out.push(GitIntent::RequestDiff {
            request: DiffRequest::local_change(file),
            job: DiffJob { target, token },
        });
    }

    /// `git.diff` : ouvre le diff du fichier sélectionné en grand.
    pub fn open_diff_window(&mut self, out: &mut Vec<GitIntent>) {
        if let Some(file) = self.selected_file().cloned() {
            self.diff_intent(&file, DiffTarget::Window, out);
        }
    }

    pub fn conflicts_keep(&mut self, side: ConflictSide, wanted: Option<&str>) -> bool {
        let Some(status) = &self.status else {
            return false;
        };
        let files: Vec<(PathBuf, bool)> = status
            .conflicts()
            .filter(|f| wanted.is_none_or(|w| f.repo_path == w.replace('\\', "/")))
            .map(|f| (f.path.clone(), side_deletes(f.conflict, side)))
            .collect();
        if files.is_empty() {
            return false;
        }
        self.confirm = Some(Confirm::KeepSide { side, files });
        true
    }

    pub fn ask_abort(&mut self) -> bool {
        match self.status.as_ref().and_then(|s| s.operation.as_ref()) {
            Some(operation) => {
                self.confirm = Some(Confirm::AbortOperation(operation.kind));
                true
            }
            None => false,
        }
    }

    pub fn answer_confirm(&mut self, yes: bool, out: &mut Vec<GitIntent>) {
        let Some(confirm) = self.confirm.take() else {
            return;
        };
        if !yes {
            return;
        }
        match confirm {
            Confirm::KeepSide { side, files } => out.push(GitIntent::AcceptSide {
                paths: files.into_iter().map(|(p, _)| p).collect(),
                side,
            }),
            Confirm::AbortOperation(_) => out.push(GitIntent::AbortOperation),
        }
    }

    pub fn dismiss_error(&mut self) {
        self.commit_error = None;
    }

    pub fn select_log(&mut self, index: usize, out: &mut Vec<GitIntent>) -> bool {
        let Some(commit) = self.log.commits.get(index) else {
            return false;
        };
        let id = commit.id.clone();
        self.log.selected = Some(index);
        self.log_focus_request = true;
        self.log.details = None;
        self.log.file_selected = None;
        self.log.details_for = Some(id.clone());
        out.push(GitIntent::LoadCommit(id));
        true
    }

    pub fn open_commit_file(&mut self, index: usize, out: &mut Vec<GitIntent>) -> bool {
        let Some(details) = &self.log.details else {
            return false;
        };
        let Some(file) = details.files.get(index) else {
            return false;
        };
        out.push(GitIntent::RequestDiff {
            request: DiffRequest::commit_file(&details.summary.id, file),
            job: DiffJob {
                target: DiffTarget::Window,
                token: 0,
            },
        });
        true
    }

    // ----- Affichage -------------------------------------------------------------

    pub fn show(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        state: RepoState,
        commit_shortcut: &str,
    ) -> Vec<GitIntent> {
        let mut out = Vec::new();
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        match state {
            RepoState::Unknown => {
                kit::centered_note(ui, rect, t::GIT_LOADING, p);
                return out;
            }
            RepoState::NoRepository => {
                kit::centered_note(ui, rect, t::GIT_NO_REPOSITORY, p);
                return out;
            }
            RepoState::Ready => {}
        }
        if self.preview_dirty {
            self.preview_dirty = false;
            match self.selected_file().cloned() {
                Some(file) => self.diff_intent(&file, DiffTarget::Preview, &mut out),
                None => self.preview.clear(),
            }
        }

        let mut top = rect.top();
        if let Some(status) = self.status.clone() {
            if let Some(operation) = &status.operation {
                let banner = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0));
                self.show_banner(ui, banner, p, &status, operation.kind, &mut out);
                top += 34.0;
            }
        }
        // Barre d'onglets Git.
        let bar = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), TAB_BAR));
        self.show_tab_bar(ui, bar, p, &mut out);
        let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
        match self.tab {
            GitTab::Changes => self.show_changes(ui, body, p, commit_shortcut, &mut out),
            GitTab::Log => self.show_log(ui, body, p, &mut out),
            GitTab::Console => self.show_console(ui, body, p),
        }
        self.show_modals(ui.ctx(), p, &mut out);
        out
    }

    fn show_banner(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        status: &Status,
        kind: OperationKind,
        out: &mut Vec<GitIntent>,
    ) {
        let conflicts = status.conflicts().count();
        let fill = if conflicts > 0 {
            p.error.gamma_multiply(0.25)
        } else {
            p.ok.gamma_multiply(0.22)
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        let text = if conflicts > 0 {
            t::operation_conflicts(kind, conflicts)
        } else {
            t::OPERATION_RESOLVED.to_owned()
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0)),
            if conflicts > 0 {
                Icon::Warning
            } else {
                Icon::Check
            },
            if conflicts > 0 { p.error } else { p.ok },
        );
        ui.painter().text(
            pos2(rect.left() + 34.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(14.0),
            p.text,
        );
        kit::child(ui, rect.shrink2(vec2(10.0, 4.0)), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::button(ui, p, t::OPERATION_ABORT, false, true).clicked() {
                    self.ask_abort();
                }
                if conflicts > 0 && kit::button(ui, p, t::OPERATION_RESOLVE, true, true).clicked() {
                    self.tab = GitTab::Changes;
                    if let Some(first) = status.conflicts().next() {
                        self.select(first.path.clone());
                    }
                }
            });
        });
        let _ = out;
    }

    fn show_tab_bar(&mut self, ui: &mut Ui, rect: Rect, p: &Palette, out: &mut Vec<GitIntent>) {
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
        let mut x = rect.left() + 6.0;
        for tab in GitTab::ALL {
            let label = tab.label();
            let galley = ui.painter().layout_no_wrap(
                label.to_owned(),
                FontId::proportional(13.0),
                if tab == self.tab { p.text } else { p.text_dim },
            );
            let width = galley.size().x + 22.0;
            let tab_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(width, rect.height()));
            let response = kit::hit(ui, tab_rect, Id::unique(("git-onglet", tab.key())));
            if tab == self.tab {
                ui.painter().rect_filled(
                    Rect::from_min_size(
                        pos2(tab_rect.left() + 6.0, tab_rect.bottom() - 2.0),
                        vec2(width - 12.0, 2.0),
                    ),
                    1.0,
                    p.accent,
                );
            } else if response.hovered() {
                ui.painter()
                    .rect_filled(tab_rect.shrink2(vec2(2.0, 3.0)), 4.0, p.bg);
            }
            ui.painter().galley(
                pos2(
                    tab_rect.left() + 11.0,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                p.text,
            );
            if response.clicked() {
                self.tab = tab;
            }
            x += width + 2.0;
        }
        if let Some(status) = &self.status {
            let text = t::git_head_summary(&status.head, status.upstream.as_ref());
            ui.painter().text(
                pos2(rect.right() - 40.0, rect.center().y),
                Align2::RIGHT_CENTER,
                text,
                FontId::proportional(12.5),
                p.text_dim,
            );
        }
        let refresh =
            Rect::from_center_size(pos2(rect.right() - 18.0, rect.center().y), vec2(22.0, 22.0));
        if icons::tool_button(
            ui,
            refresh,
            Id::unique("git-actualiser"),
            Icon::Refresh,
            p,
            t::GIT_REFRESH,
            true,
        ) {
            out.push(GitIntent::Refresh);
            self.preview_dirty = true;
            if self.tab == GitTab::Log {
                self.reset_log();
            }
        }
    }

    // ----- Onglet Changements ----------------------------------------------------

    fn show_changes(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        commit_shortcut: &str,
        out: &mut Vec<GitIntent>,
    ) {
        let Some(status) = self.status.clone() else {
            kit::centered_note(ui, rect, t::GIT_LOADING, p);
            return;
        };
        let left_width = (rect.width() * 0.42)
            .clamp(300.0, 560.0)
            .min(rect.width() - 200.0);
        let left = Rect::from_min_size(rect.min, vec2(left_width, rect.height()));
        let right = Rect::from_min_max(pos2(left.right() + 1.0, rect.top()), rect.max);
        ui.painter().vline(
            left.right(),
            rect.y_range(),
            egui::Stroke::new(1.0, p.border),
        );

        // Zone de message en bas de la colonne de gauche.
        let operation = status.operation.is_some();
        let message_height = 112.0;
        let list_rect =
            Rect::from_min_max(left.min, pos2(left.right(), left.bottom() - message_height));
        let message_rect = Rect::from_min_max(pos2(left.left(), list_rect.bottom()), left.max);
        self.show_file_list(ui, list_rect, p, &status, out);
        self.show_commit_box(
            ui,
            message_rect,
            p,
            &status,
            operation,
            commit_shortcut,
            out,
        );

        // Aperçu du diff.
        match self.selected_file() {
            Some(file) if file.group() == Group::Conflicted => {
                kit::centered_note(ui, right, t::GIT_CONFLICT_HINT, p);
            }
            Some(_) if self.preview.is_loaded() => {
                let keys = ui.rect_contains_pointer(right);
                self.preview.show(ui, right, p, keys);
            }
            Some(_) => kit::centered_note(ui, right, t::DIFF_LOADING, p),
            None => {
                let text = if status.files.is_empty() {
                    t::GIT_CLEAN
                } else {
                    t::DIFF_NO_SELECTION
                };
                kit::centered_note(ui, right, text, p);
            }
        }
    }

    fn show_file_list(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        status: &Arc<Status>,
        out: &mut Vec<GitIntent>,
    ) {
        if status.files.is_empty() {
            kit::centered_note(ui, rect, t::GIT_CLEAN, p);
            return;
        }
        // Clavier : la liste garde le focus après un clic.
        let focused = ui.memory(|m| m.has_focus(self.list_focus));
        ui.interact(rect, self.list_focus, Sense::click());
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    self.list_focus,
                    EventFilter {
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
            self.list_keys(ui, status, out);
        }
        let operation = status.operation.is_some();
        let groups = [
            (Group::Conflicted, t::GIT_GROUP_CONFLICTS),
            (Group::Changed, t::GIT_GROUP_CHANGED),
            (Group::Untracked, t::GIT_GROUP_UNTRACKED),
        ];
        let mut clicked: Option<PathBuf> = None;
        let mut double: Option<PathBuf> = None;
        let mut toggled: Vec<(PathBuf, bool)> = Vec::new();
        let mut merge_request: Option<PathBuf> = None;
        let mut keep: Option<(ConflictSide, PathBuf)> = None;
        kit::child(ui, rect, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt("git-fichiers")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(rect.width() - 12.0);
                    for (group, title) in groups {
                        let files: Vec<&FileStatus> =
                            status.files.iter().filter(|f| f.group() == group).collect();
                        if files.is_empty() {
                            continue;
                        }
                        let checkable = group != Group::Conflicted && !operation;
                        let all = files.iter().all(|f| self.checked.contains(&f.path));
                        let any = files.iter().any(|f| self.checked.contains(&f.path));
                        // En-tête de groupe.
                        let (header, response) = ui.allocate_exact_size(
                            vec2(ui.available_width(), ROW + 2.0),
                            Sense::click(),
                        );
                        ui.painter().rect_filled(header, 0.0, p.bg_alt);
                        let mut x = header.left() + 8.0;
                        if checkable {
                            let box_rect = Rect::from_min_size(
                                pos2(x - 4.0, header.top()),
                                vec2(22.0, header.height()),
                            );
                            kit::paint_checkbox(ui, box_rect, all, any && !all, p);
                            if response.clicked() {
                                for f in &files {
                                    toggled.push((f.path.clone(), !all));
                                }
                            }
                            x += 24.0;
                        }
                        let color = if group == Group::Conflicted {
                            p.error
                        } else {
                            p.text
                        };
                        ui.painter().text(
                            pos2(x, header.center().y),
                            Align2::LEFT_CENTER,
                            format!("{title}  {}", files.len()),
                            FontId::proportional(13.0),
                            color,
                        );
                        for file in files {
                            if group == Group::Conflicted {
                                self.conflict_row(
                                    ui,
                                    p,
                                    file,
                                    &mut clicked,
                                    &mut merge_request,
                                    &mut keep,
                                );
                            } else {
                                let checked = checkable.then(|| self.checked.contains(&file.path));
                                let selected = self.selected.as_ref() == Some(&file.path);
                                let row = file_row(ui, p, file, selected, checked);
                                if row.toggled {
                                    toggled.push((file.path.clone(), !checked.unwrap_or(false)));
                                } else if row.double {
                                    double = Some(file.path.clone());
                                } else if row.clicked {
                                    clicked = Some(file.path.clone());
                                }
                            }
                        }
                    }
                });
        });
        for (path, value) in toggled {
            if value {
                self.checked.insert(path);
            } else {
                self.checked.remove(&path);
            }
        }
        if let Some(path) = clicked.or(double.clone()) {
            ui.memory_mut(|m| m.request_focus(self.list_focus));
            self.select(path);
        }
        if double.is_some() {
            self.open_diff_window(out);
        }
        if let Some(path) = merge_request {
            self.merge_loading = Some(path.clone());
            out.push(GitIntent::OpenMerge(path));
        }
        if let Some((side, path)) = keep {
            let deletes = status
                .find(&path)
                .map(|f| side_deletes(f.conflict, side))
                .unwrap_or(false);
            self.confirm = Some(Confirm::KeepSide {
                side,
                files: vec![(path, deletes)],
            });
        }
    }

    fn list_keys(&mut self, ui: &mut Ui, status: &Arc<Status>, out: &mut Vec<GitIntent>) {
        let ordered: Vec<&FileStatus> = [Group::Conflicted, Group::Changed, Group::Untracked]
            .into_iter()
            .flat_map(|g| status.files.iter().filter(move |f| f.group() == g))
            .collect();
        let position = self
            .selected
            .as_ref()
            .and_then(|s| ordered.iter().position(|f| &f.path == s));
        let down = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
        let up = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
        let space = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space));
        let enter = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let ctrl_d = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::D));
        let f4 = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F4));
        let target = match (position, down, up) {
            (None, true, _) | (None, _, true) => Some(0),
            (Some(i), true, _) if i + 1 < ordered.len() => Some(i + 1),
            (Some(i), _, true) if i > 0 => Some(i - 1),
            _ => None,
        };
        if let Some(i) = target {
            if let Some(file) = ordered.get(i) {
                self.select(file.path.clone());
            }
        }
        if space && status.operation.is_none() {
            if let Some(file) = position.and_then(|i| ordered.get(i)) {
                if file.group() != Group::Conflicted && !self.checked.remove(&file.path) {
                    self.checked.insert(file.path.clone());
                }
            }
        }
        if enter || ctrl_d {
            self.open_diff_window(out);
        }
        if f4 {
            if let Some(selected) = &self.selected {
                out.push(GitIntent::OpenFile(selected.clone()));
            }
        }
    }

    fn conflict_row(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        file: &FileStatus,
        clicked: &mut Option<PathBuf>,
        merge_request: &mut Option<PathBuf>,
        keep: &mut Option<(ConflictSide, PathBuf)>,
    ) {
        let selected = self.selected.as_ref() == Some(&file.path);
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
        if selected {
            ui.painter().rect_filled(rect, 0.0, p.accent_soft);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, p.bg_alt);
        }
        if response.clicked() {
            *clicked = Some(file.path.clone());
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(rect.left() + 16.0, rect.top() + 14.0),
                vec2(14.0, 14.0),
            ),
            Icon::Warning,
            p.git_conflict,
        );
        let (name, dir) = split_name(&file.repo_path);
        let clip = ui.painter().with_clip_rect(rect);
        let name_rect = clip.text(
            pos2(rect.left() + 30.0, rect.top() + 14.0),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(13.5),
            p.git_conflict,
        );
        clip.text(
            pos2(name_rect.right() + 8.0, rect.top() + 14.0),
            Align2::LEFT_CENTER,
            dir,
            FontId::proportional(12.0),
            p.text_dim,
        );
        let kind = file.conflict;
        if let Some(kind) = kind {
            clip.text(
                pos2(rect.right() - 8.0, rect.top() + 14.0),
                Align2::RIGHT_CENTER,
                t::conflict_kind(kind),
                FontId::proportional(12.0),
                p.text_dim,
            );
        }
        let buttons = Rect::from_min_size(
            pos2(rect.left() + 28.0, rect.top() + 26.0),
            vec2(rect.width() - 34.0, 26.0),
        );
        kit::child(ui, buttons, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (ours_label, theirs_label) = t::keep_labels(kind);
                if ui.small_button(ours_label).clicked() {
                    *keep = Some((ConflictSide::Ours, file.path.clone()));
                }
                if ui.small_button(theirs_label).clicked() {
                    *keep = Some((ConflictSide::Theirs, file.path.clone()));
                }
                let disabled = kind.and_then(merge_disabled_reason);
                let busy = self.merge_loading.as_ref() == Some(&file.path);
                let merge = ui.add_enabled(
                    disabled.is_none() && !busy,
                    egui::Button::new(RichText::new(t::MERGE_BUTTON).color(
                        if disabled.is_none() {
                            Color32::WHITE
                        } else {
                            p.text_dim
                        },
                    ))
                    .small()
                    .fill(if disabled.is_none() {
                        p.accent
                    } else {
                        p.bg_alt
                    }),
                );
                if let Some(reason) = disabled {
                    merge.on_disabled_hover_text(reason);
                } else if merge.clicked() {
                    *merge_request = Some(file.path.clone());
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn show_commit_box(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        status: &Arc<Status>,
        operation: bool,
        shortcut: &str,
        out: &mut Vec<GitIntent>,
    ) {
        ui.painter()
            .hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, p.border));
        let inner = rect.shrink2(vec2(8.0, 6.0));
        // Ctrl+Entrée dans le message = Valider.
        let message_focused = ui.memory(|m| m.has_focus(self.message_id));
        let ctrl_enter =
            message_focused && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter));
        kit::child(ui, inner, |ui| {
            let edit = ui.add(
                TextEdit::multiline(&mut self.message)
                    .id(self.message_id)
                    .hint_text(t::COMMIT_HINT)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .font(FontId::proportional(13.5)),
            );
            if self.focus_message {
                self.focus_message = false;
                edit.request_focus();
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let label = if operation {
                    t::COMMIT_CONCLUDE
                } else if self.amend {
                    t::COMMIT_AMEND_BUTTON
                } else {
                    t::COMMIT_BUTTON
                };
                let enabled = self.can_commit();
                let button = kit::button(ui, p, label, true, enabled)
                    .on_hover_text(t::commit_tooltip(shortcut));
                if button.clicked() {
                    if let Some(intent) = self.commit_intent() {
                        out.push(intent);
                    }
                }
                if !operation {
                    ui.checkbox(&mut self.amend, t::COMMIT_AMEND);
                }
                let selected = status
                    .files
                    .iter()
                    .filter(|f| f.group() != Group::Conflicted && self.checked.contains(&f.path))
                    .count();
                let text = if operation {
                    String::new()
                } else {
                    t::commit_selected(selected)
                };
                ui.label(RichText::new(text).color(p.text_dim).small());
            });
        });
        if ctrl_enter {
            if let Some(intent) = self.commit_intent() {
                out.push(intent);
            }
        }
    }

    // ----- Onglet Journal --------------------------------------------------------

    fn show_log(&mut self, ui: &mut Ui, rect: Rect, p: &Palette, out: &mut Vec<GitIntent>) {
        if !self.log.requested {
            self.log.requested = true;
            self.log.loading = true;
            out.push(GitIntent::LoadLog(LogQuery::default()));
        }
        if self.log.commits.is_empty() {
            let text = if self.log.loading {
                t::GIT_LOADING
            } else {
                t::LOG_EMPTY
            };
            kit::centered_note(ui, rect, text, p);
            return;
        }
        let left_width = (rect.width() * 0.62).max(300.0).min(rect.width() - 160.0);
        let left = Rect::from_min_size(rect.min, vec2(left_width, rect.height()));
        let right = Rect::from_min_max(pos2(left.right() + 1.0, rect.top()), rect.max);
        ui.painter().vline(
            left.right(),
            rect.y_range(),
            egui::Stroke::new(1.0, p.border),
        );

        let count = self.log.commits.len();
        let mut clicked = None;
        let mut want_more = false;
        // Clavier : la liste garde le focus après un clic ; flèches, pages, Début, Fin.
        let focus = Id::unique("git-journal-focus");
        ui.interact(left, focus, Sense::click());
        if std::mem::take(&mut self.log_focus_request) {
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
            let current = self.log.selected;
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
        let view_height = left.height();
        kit::child(ui, left, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut area = egui::ScrollArea::vertical()
                .id_salt("git-journal")
                .auto_shrink([false, false]);
            if let Some(i) = scroll_to {
                area = area.vertical_scroll_offset((i as f32 * ROW - view_height / 2.0).max(0.0));
            }
            area.show_rows(ui, ROW, count, |ui, range| {
                if range.end + 20 >= count && self.log.has_more && !self.log.loading {
                    want_more = true;
                }
                for i in range {
                    let selected = self.log.selected == Some(i);
                    if log_row(ui, p, &self.log.commits[i], selected) {
                        clicked = Some(i);
                        ui.memory_mut(|m| m.request_focus(focus));
                    }
                }
            });
        });
        if want_more {
            self.log.loading = true;
            out.push(GitIntent::LoadLog(LogQuery {
                skip: count,
                ..LogQuery::default()
            }));
        }
        if let Some(i) = clicked {
            self.select_log(i, out);
        }
        self.show_commit_details(ui, right, p, out);
    }

    fn show_commit_details(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        out: &mut Vec<GitIntent>,
    ) {
        let Some(index) = self.log.selected else {
            kit::centered_note(ui, rect, t::LOG_SELECT, p);
            return;
        };
        let Some(summary) = self.log.commits.get(index).cloned() else {
            return;
        };
        let details = self.log.details.clone();
        let mut open_file: Option<usize> = None;
        let mut select_file: Option<usize> = None;
        let selected_file = self.log.file_selected;
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
            self.log.file_selected = Some(i);
        }
        if let Some(i) = open_file {
            self.log.file_selected = Some(i);
            self.open_commit_file(i, out);
        }
    }

    // ----- Onglet Console --------------------------------------------------------

    fn show_console(&mut self, ui: &mut Ui, rect: Rect, p: &Palette) {
        if self.console.is_empty() {
            kit::centered_note(ui, rect, t::CONSOLE_EMPTY, p);
            return;
        }
        kit::child(ui, rect.shrink2(vec2(8.0, 4.0)), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("git-console")
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.set_width(rect.width() - 24.0);
                    for record in &self.console {
                        let failed = record
                            .result
                            .as_ref()
                            .is_some_and(|r| r.exit_code != Some(0));
                        let open = self.console_open.contains(&record.id) || failed;
                        let (summary, ok) = console_summary(record);
                        let header = ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
                            ui.painter().circle_filled(
                                dot.center(),
                                3.5,
                                match ok {
                                    Some(true) => p.ok,
                                    Some(false) => p.error,
                                    None => p.warn,
                                },
                            );
                            ui.label(
                                RichText::new(format!("git {}", record.args.join(" ")))
                                    .monospace()
                                    .color(p.text),
                            );
                            ui.label(RichText::new(summary).small().color(p.text_dim));
                        });
                        if header.response.interact(Sense::click()).clicked()
                            && !self.console_open.remove(&record.id)
                        {
                            self.console_open.insert(record.id);
                        }
                        if open {
                            if let Some(result) = &record.result {
                                for text in [&result.stdout, &result.stderr] {
                                    if !text.trim().is_empty() {
                                        let shown: Vec<&str> =
                                            text.trim_end().lines().take(30).collect();
                                        let more = text
                                            .trim_end()
                                            .lines()
                                            .count()
                                            .saturating_sub(shown.len());
                                        let mut text = shown.join(
                                            "
",
                                        );
                                        if more > 0 {
                                            text.push_str(&format!(
                                                "
… {more} lignes de plus"
                                            ));
                                        }
                                        ui.label(
                                            RichText::new(text)
                                                .monospace()
                                                .small()
                                                .color(p.text_dim),
                                        );
                                    }
                                }
                            }
                        }
                    }
                });
        });
    }

    // ----- Fenêtres modales ------------------------------------------------------

    fn show_modals(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<GitIntent>) {
        if let Some(confirm) = self.confirm.clone() {
            let (title, detail) = match &confirm {
                Confirm::KeepSide { side, files } => {
                    let names: Vec<String> = files.iter().map(|(f, _)| file_name(f)).collect();
                    let deletes: Vec<String> = files
                        .iter()
                        .filter(|(_, d)| *d)
                        .map(|(f, _)| file_name(f))
                        .collect();
                    (
                        t::confirm_keep_title(*side, &names),
                        t::confirm_keep_detail(*side, &deletes),
                    )
                }
                Confirm::AbortOperation(kind) => (
                    t::confirm_abort_title(*kind),
                    t::CONFIRM_ABORT_DETAIL.to_owned(),
                ),
            };
            let mut answer = None;
            kit::modal("git-confirmation").show(ctx, |ui| {
                ui.set_width(430.0);
                ui.label(RichText::new(title).strong().size(15.0).color(p.text));
                ui.add_space(4.0);
                ui.label(RichText::new(detail).color(p.text_dim));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if kit::button(ui, p, t::CONFIRM_YES, true, true).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(t::DIALOG_CANCEL).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                answer = Some(false);
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                answer = Some(true);
            }
            if let Some(yes) = answer {
                self.answer_confirm(yes, out);
            }
        }
        if let Some(text) = self.commit_error.clone() {
            let mut close = false;
            kit::modal("git-echec-commit").show(ctx, |ui| {
                ui.set_width(620.0);
                ui.label(
                    RichText::new(t::COMMIT_FAILED)
                        .strong()
                        .size(15.0)
                        .color(p.error),
                );
                ui.label(RichText::new(t::COMMIT_FAILED_DETAIL).color(p.text_dim));
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(RichText::new(text).monospace().small().color(p.text));
                    });
                ui.add_space(10.0);
                if ui.button(t::CLOSE).clicked() {
                    close = true;
                }
            });
            if close || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.commit_error = None;
            }
        }
    }

    // ----- État pour le mode scénario --------------------------------------------

    pub fn state(&self) -> serde_json::Value {
        let files = self.status.as_ref().map(|s| {
            s.files
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "path": f.repo_path,
                        "kind": format!("{:?}", f.kind),
                        "group": format!("{:?}", f.group()),
                        "conflict": f.conflict.map(|c| format!("{c:?}")),
                        "checked": self.checked.contains(&f.path),
                        "selected": self.selected.as_ref() == Some(&f.path),
                    })
                })
                .collect::<Vec<_>>()
        });
        serde_json::json!({
            "tab": self.tab.key(),
            "files": files,
            "operation": self.status.as_ref().and_then(|s| s.operation.as_ref()).map(|o| format!("{:?}", o.kind)),
            "message": self.message,
            "amend": self.amend,
            "commit_pending": self.commit_pending,
            "commit_error": self.commit_error,
            "confirm": self.confirm.as_ref().map(|c| match c {
                Confirm::KeepSide { side, files } => format!("keep_{side:?}:{}", files.len()),
                Confirm::AbortOperation(k) => format!("abort:{k:?}"),
            }),
            "preview": self.preview.state(),
            "log": {
                "loaded": self.log.commits.len(),
                "has_more": self.log.has_more,
                "loading": self.log.loading,
                "selected": self.log.selected,
                "subjects": self.log.commits.iter().take(30).map(|c| c.subject.clone()).collect::<Vec<_>>(),
                "files": self.log.details.as_ref().map(|d| d.files.iter().map(|f| f.repo_path.clone()).collect::<Vec<_>>()),
                "message": self.log.details.as_ref().map(|d| d.message.clone()),
            },
            "console": {
                "commands": self.console.len(),
                "last": self.console.last().map(|r| format!("git {}", r.args.join(" "))),
            },
        })
    }
}

// ----- Dessin des lignes -------------------------------------------------------

struct RowEvent {
    clicked: bool,
    double: bool,
    toggled: bool,
}

fn kind_color(kind: FileKind, p: &Palette) -> Color32 {
    match kind {
        FileKind::Modified | FileKind::Renamed | FileKind::TypeChanged => p.git_modified,
        FileKind::Added | FileKind::Copied => p.git_added,
        FileKind::Deleted => p.git_deleted,
        FileKind::Untracked => p.git_untracked,
        FileKind::Conflicted => p.git_conflict,
    }
}

fn split_name(repo_path: &str) -> (&str, &str) {
    match repo_path.rsplit_once('/') {
        Some((dir, name)) => (name, dir),
        None => (repo_path, ""),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn file_row(
    ui: &mut Ui,
    p: &Palette,
    file: &FileStatus,
    selected: bool,
    checked: Option<bool>,
) -> RowEvent {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
    }
    let mut x = rect.left() + 8.0;
    let mut toggled = false;
    if let Some(checked) = checked {
        let box_rect = Rect::from_min_size(pos2(x - 4.0, rect.top()), vec2(22.0, ROW));
        let hit = ui.interact(
            box_rect,
            Id::unique(("git-case", &file.repo_path)),
            Sense::click(),
        );
        kit::paint_checkbox(ui, box_rect, checked, false, p);
        toggled = hit.clicked();
        x += 24.0;
    }
    let color = kind_color(file.kind, p);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(x + 7.0, rect.center().y), vec2(13.0, 13.0)),
        Icon::File,
        color,
    );
    x += 20.0;
    let clip = ui.painter().with_clip_rect(rect);
    let (name, dir) = split_name(&file.repo_path);
    let name_text = match &file.original_repo_path {
        Some(original) => format!("{} → {}", split_name(original).0, name),
        None => name.to_owned(),
    };
    let name_rect = clip.text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        name_text,
        FontId::proportional(13.5),
        color,
    );
    clip.text(
        pos2(name_rect.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        dir,
        FontId::proportional(12.0),
        p.text_dim,
    );
    clip.text(
        pos2(rect.right() - 8.0, rect.center().y),
        Align2::RIGHT_CENTER,
        t::file_kind(file.kind),
        FontId::proportional(11.5),
        p.text_dim,
    );
    RowEvent {
        clicked: response.clicked() && !toggled,
        double: response.double_clicked(),
        toggled,
    }
}

fn log_row(ui: &mut Ui, p: &Palette, commit: &CommitSummary, selected: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, p.accent_soft);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, p.bg_alt);
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
    let mut x = rect.left() + 10.0;
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
    response.clicked()
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
    let (name, dir) = split_name(&file.repo_path);
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

fn console_summary(record: &CommandRecord) -> (String, Option<bool>) {
    match &record.result {
        None => (t::CONSOLE_RUNNING.to_owned(), None),
        Some(CommandResult {
            duration,
            exit_code,
            ..
        }) => (
            format!(
                "{} — {}",
                kit::format_duration(*duration),
                match exit_code {
                    Some(0) => t::CONSOLE_OK.to_owned(),
                    Some(code) => t::console_exit(*code),
                    None => t::CONSOLE_NO_CODE.to_owned(),
                }
            ),
            exit_code.map(|c| c == 0),
        ),
    }
}

/// La sortie de Git telle quelle (crochets compris) pour un échec.
pub fn failure_text(error: &VcsError) -> String {
    match error {
        VcsError::GitFailed { stdout, stderr, .. } => {
            let mut text = String::new();
            for part in [stdout, stderr] {
                if !part.trim().is_empty() {
                    text.push_str(part.trim_end());
                    text.push('\n');
                }
            }
            if text.is_empty() {
                error.to_string()
            } else {
                text
            }
        }
        other => other.to_string(),
    }
}

/// `.git/MERGE_MSG` sans les lignes de commentaire (`# Conflicts:`…) que Git ne
/// retire pas quand le message est donné par `-F`.
pub fn clean_merge_message(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
    lines
        .join(
            "
",
        )
        .trim()
        .to_owned()
}

/// Garder ce côté supprime-t-il le fichier ?
pub fn side_deletes(kind: Option<ConflictKind>, side: ConflictSide) -> bool {
    matches!(
        (kind, side),
        (Some(ConflictKind::DeletedByUs), ConflictSide::Ours)
            | (Some(ConflictKind::DeletedByThem), ConflictSide::Theirs)
            | (Some(ConflictKind::AddedByUs), ConflictSide::Theirs)
            | (Some(ConflictKind::AddedByThem), ConflictSide::Ours)
            | (Some(ConflictKind::BothDeleted), _)
    )
}

/// Pourquoi « Fusionner… » est grisé, s'il l'est.
pub fn merge_disabled_reason(kind: ConflictKind) -> Option<&'static str> {
    if kind.involves_deletion() {
        Some(t::MERGE_DISABLED_DELETION)
    } else if matches!(kind, ConflictKind::AddedByUs | ConflictKind::AddedByThem) {
        Some(t::MERGE_DISABLED_ONE_SIDE)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeping_a_side_that_deleted_the_file_is_a_deletion() {
        assert!(side_deletes(
            Some(ConflictKind::DeletedByThem),
            ConflictSide::Theirs
        ));
        assert!(!side_deletes(
            Some(ConflictKind::DeletedByThem),
            ConflictSide::Ours
        ));
        assert!(side_deletes(
            Some(ConflictKind::DeletedByUs),
            ConflictSide::Ours
        ));
        assert!(!side_deletes(
            Some(ConflictKind::BothModified),
            ConflictSide::Theirs
        ));
    }

    #[test]
    fn merge_message_loses_its_git_comments() {
        let text = "Merge branch 'x' into y

# Conflicts:
#	README.md
";
        assert_eq!(clean_merge_message(text), "Merge branch 'x' into y");
    }

    #[test]
    fn deletion_conflicts_cannot_be_merged() {
        assert!(merge_disabled_reason(ConflictKind::DeletedByThem).is_some());
        assert!(merge_disabled_reason(ConflictKind::AddedByUs).is_some());
        assert!(merge_disabled_reason(ConflictKind::BothModified).is_none());
        assert!(merge_disabled_reason(ConflictKind::BothAdded).is_none());
    }

    #[test]
    fn git_failures_are_shown_as_git_wrote_them() {
        let error = VcsError::GitFailed {
            command: "commit".into(),
            code: Some(1),
            stdout: "hook: échec\n".into(),
            stderr: "error: la validation est refusée\n".into(),
        };
        assert_eq!(
            failure_text(&error),
            "hook: échec\nerror: la validation est refusée\n"
        );
    }
}
