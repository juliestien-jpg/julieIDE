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
    BranchList, CommandRecord, CommandResult, CommitRequest, ConflictKind, ConflictLabels,
    ConflictSide, DetachedHeadReport, DiffRequest, FileKind, FileStatus, Group, IndexState,
    LogQuery, OperationKind, PendingStash, PendingStashState, Status, VcsError, VcsEvent,
};

use crate::diffpane::DiffPane;
use crate::icons::{self, Icon};
use crate::kit;
use crate::logview::{Filters, LogView};
use crate::merge::Ineligible;
use crate::target::{FileRef, MenuRequest, Target, Zone};
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
    SkipOperation,
    AcceptSide {
        paths: Vec<PathBuf>,
        side: ConflictSide,
    },
    OpenMerge(PathBuf),
    LoadLog(LogQuery),
    LoadCommit(String),
    Notify(String),
    /// « Remettre mes changements » : la mise de côté liée à une opération terminée ailleurs.
    RestorePendingStash,
    /// « Remettre » une mise de côté gardée (son commit).
    RestoreStash(String),
    /// « Créer une branche ici » (bandeau de la HEAD détachée).
    NewBranchHere,
}

/// Question posée avant une action qui écrase du travail.
#[derive(Debug, Clone)]
pub enum Confirm {
    KeepSide {
        side: ConflictSide,
        files: Vec<(PathBuf, bool)>,
    },
    AbortOperation(OperationKind),
    SkipOperation(OperationKind),
}

/// Ce que l'application sait du dépôt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    Unknown,
    NoRepository,
    Ready,
}

/// Ce que l'on sait d'avance d'un fichier en conflit pour le bouton « Fusionner… ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeCheck {
    /// Les versions se chargent en tâche de fond.
    Loading,
    /// Le dialogue peut s'ouvrir.
    Fine,
    /// Le dialogue est impossible, pour cette raison.
    Refused(Ineligible),
    /// Le chargement a échoué : on ne grise pas, le refus éventuel viendra au clic.
    Unknown,
}

/// Nombre de chargements de versions en même temps (les gros fichiers pèsent en mémoire).
const MAX_PRECHECKS: usize = 2;

pub struct GitView {
    pub tab: GitTab,
    status: Option<Arc<Status>>,
    checked: HashSet<PathBuf>,
    known: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
    /// Sélection multiple (Ctrl+clic, Maj+clic) : tous les fichiers choisis quand ils sont
    /// plusieurs, `selected` compris. Vide : la sélection est `selected` seul.
    multi: HashSet<PathBuf>,
    anchor: Option<PathBuf>,
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
    pub log: LogView,
    console: Vec<CommandRecord>,
    console_open: HashSet<u64>,
    pub confirm: Option<Confirm>,
    /// Menu contextuel demandé à cette image (liste des changements, journal).
    pub menu_request: Option<MenuRequest>,
    list_focus: Id,
    message_id: Id,
    /// Fichier dont la fusion se prépare (chargement des versions).
    pub merge_loading: Option<PathBuf>,
    /// Vérification d'avance de « Fusionner… », par fichier en conflit.
    merge_checks: HashMap<PathBuf, MergeCheck>,
    /// Tâches `vcs.conflict` lancées pour cette vérification (et non pour ouvrir le dialogue).
    check_tasks: HashMap<TaskId, PathBuf>,
    /// Mises de côté de l'IDE encore dans `git stash list` (bandeau).
    pending_stashes: Vec<PendingStash>,
    /// HEAD détachée : commits sur aucune branche (bandeau).
    detached: Option<DetachedHeadReport>,
    /// Qui est qui dans les conflits, avec les noms lus par la vérification d'avance
    /// (`None` : on ne sait que les rôles, d'après l'opération).
    labels: Option<(Option<OperationKind>, ConflictLabels)>,
    /// Opération et étape de l'état précédent : les noms changent quand l'étape change.
    operation_key: Option<(OperationKind, Option<(u32, u32)>)>,
}

impl GitView {
    pub fn new(dark: bool) -> Self {
        Self {
            tab: GitTab::Changes,
            status: None,
            checked: HashSet::new(),
            known: HashSet::new(),
            selected: None,
            multi: HashSet::new(),
            anchor: None,
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
            log: LogView::default(),
            console: Vec::new(),
            console_open: HashSet::new(),
            confirm: None,
            menu_request: None,
            list_focus: Id::unique("git-liste"),
            message_id: Id::unique("git-message"),
            merge_loading: None,
            merge_checks: HashMap::new(),
            check_tasks: HashMap::new(),
            pending_stashes: Vec::new(),
            detached: None,
            labels: None,
            operation_key: None,
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
        let conflicted: HashSet<&PathBuf> = status.conflicts().map(|f| &f.path).collect();
        let operation_key = status.operation.as_ref().map(|o| (o.kind, o.step));
        if operation_key != self.operation_key {
            // Autre opération ou autre étape d'un rebase : les noms des côtés sont à relire.
            self.operation_key = operation_key;
            self.labels = None;
            self.merge_checks.clear();
            self.check_tasks.clear();
        }
        if matches!(status.head, julie_vcs::Head::Branch { .. }) {
            self.detached = None;
        }
        self.merge_checks.retain(|p, _| conflicted.contains(p));
        self.check_tasks.retain(|_, p| conflicted.contains(p));
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

    /// Fichiers en conflit dont il faut encore vérifier d'avance la fusion (au plus
    /// `MAX_PRECHECKS` à la fois) ; ils passent à « en chargement ».
    pub fn next_prechecks(&mut self) -> Vec<PathBuf> {
        let Some(status) = self.status.clone() else {
            return Vec::new();
        };
        let mut loading = self
            .merge_checks
            .values()
            .filter(|c| **c == MergeCheck::Loading)
            .count();
        let mut wanted = Vec::new();
        for file in status.conflicts() {
            if loading >= MAX_PRECHECKS {
                break;
            }
            let decided = file
                .conflict
                .is_none_or(|k| merge_disabled_reason(k).is_some());
            if decided || self.merge_checks.contains_key(&file.path) {
                continue;
            }
            self.merge_checks
                .insert(file.path.clone(), MergeCheck::Loading);
            loading += 1;
            wanted.push(file.path.clone());
        }
        wanted
    }

    /// Les noms des côtés, lus avec les versions d'un fichier en conflit.
    pub fn set_labels(&mut self, kind: Option<OperationKind>, labels: ConflictLabels) {
        self.labels = Some((kind, labels));
    }

    /// Qui est qui dans les conflits de l'état courant : les noms lus s'ils sont de la bonne
    /// opération, sinon les seuls rôles.
    pub fn labels(&self) -> ConflictLabels {
        let kind = self
            .status
            .as_ref()
            .and_then(|s| s.operation.as_ref())
            .map(|o| o.kind);
        match &self.labels {
            Some((k, labels)) if *k == kind => labels.clone(),
            _ => ConflictLabels::for_operation(kind),
        }
    }

    pub fn set_pending_stashes(&mut self, list: Vec<PendingStash>) {
        self.pending_stashes = list;
    }

    pub fn set_detached(&mut self, report: Option<DetachedHeadReport>) {
        self.detached = report;
    }

    /// La mise de côté la plus à dire dans le bandeau : celle qui attend l'opération en cours,
    /// puis celle d'une opération finie ailleurs, puis une gardée.
    fn main_pending(&self) -> Option<&PendingStash> {
        let rank = |s: &PendingStash| match s.state {
            PendingStashState::AwaitingOperation(_) => 0,
            PendingStashState::OperationOver => 1,
            PendingStashState::Kept => 2,
        };
        self.pending_stashes.iter().min_by_key(|s| rank(s))
    }

    /// Texte du bandeau des mises de côté, s'il y en a.
    fn stash_banner_text(&self) -> Option<String> {
        let main = self.main_pending()?;
        let more = self.pending_stashes.len() - 1;
        Some(match main.state {
            PendingStashState::AwaitingOperation(kind) => {
                t::banner_stash_waiting(kind, &main.message, more)
            }
            PendingStashState::OperationOver => t::banner_stash_over(&main.message, more),
            PendingStashState::Kept => t::banner_stash_kept(&main.message, more),
        })
    }

    /// Ce que fait le bouton d'un bandeau (scénario) : `stash` ou `branch`.
    pub fn banner_intent(&self, which: &str) -> Option<GitIntent> {
        match which {
            "stash" => self.main_pending().and_then(|m| match m.state {
                PendingStashState::AwaitingOperation(_) => None,
                PendingStashState::OperationOver => Some(GitIntent::RestorePendingStash),
                PendingStashState::Kept => Some(GitIntent::RestoreStash(m.entry.commit.clone())),
            }),
            "branch" => self
                .detached_banner_text()
                .map(|_| GitIntent::NewBranchHere),
            _ => None,
        }
    }

    /// Texte du bandeau de la HEAD détachée, s'il y a des commits sur aucune branche.
    fn detached_banner_text(&self) -> Option<String> {
        self.detached
            .as_ref()
            .filter(|d| d.orphan_commits > 0)
            .map(|d| t::banner_detached(d.orphan_commits))
    }

    pub fn note_precheck_task(&mut self, task: TaskId, path: PathBuf) {
        self.check_tasks.insert(task, path);
    }

    /// Cette tâche est-elle une vérification d'avance ? Rend le fichier concerné.
    pub fn precheck_path(&self, task: Option<TaskId>) -> Option<PathBuf> {
        self.check_tasks.get(&task?).cloned()
    }

    pub fn finish_precheck(&mut self, task: Option<TaskId>) {
        if let Some(task) = task {
            self.check_tasks.remove(&task);
        }
    }

    pub fn set_merge_check(&mut self, path: &Path, check: MergeCheck) {
        if let Some(slot) = self.merge_checks.get_mut(path) {
            *slot = check;
        }
    }

    pub fn merge_check(&self, path: &Path) -> Option<MergeCheck> {
        self.merge_checks.get(path).copied()
    }

    /// Pourquoi « Fusionner… » est grisé pour ce fichier, s'il l'est.
    fn merge_disabled(&self, file: &FileStatus) -> Option<String> {
        if let Some(reason) = file.conflict.and_then(merge_disabled_reason) {
            return Some(reason.to_owned());
        }
        match self.merge_checks.get(&file.path) {
            Some(MergeCheck::Refused(why)) => Some(t::merge_disabled_tip(why.reason())),
            _ => None,
        }
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
            VcsEvent::Log(page) => self.log.on_page(page),
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
                self.log.reset();
                out.push(GitIntent::Notify(t::commit_done(&outcome.subject)));
            }
            VcsEvent::OperationContinued(outcome) => {
                self.commit_pending = false;
                if outcome.finished {
                    self.message.clear();
                    self.merge_prefilled = false;
                    self.log.reset();
                    out.push(GitIntent::Notify(t::operation_done(outcome.kind)));
                }
            }
            VcsEvent::OperationAborted(_) => {
                self.message.clear();
                self.merge_prefilled = false;
                self.log.reset();
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

    /// Les fichiers choisis, dans l'ordre de la liste (un seul sans sélection multiple).
    pub fn selection_paths(&self) -> Vec<PathBuf> {
        let Some(status) = &self.status else {
            return self.selected.iter().cloned().collect();
        };
        if self.multi.len() > 1 {
            return self
                .ordered_paths(status)
                .into_iter()
                .filter(|p| self.multi.contains(p))
                .collect();
        }
        self.selected.iter().cloned().collect()
    }

    /// Les fichiers sans conflit, dans l'ordre d'affichage.
    fn ordered_paths(&self, status: &Status) -> Vec<PathBuf> {
        [Group::Changed, Group::Untracked]
            .into_iter()
            .flat_map(|g| status.files.iter().filter(move |f| f.group() == g))
            .map(|f| f.path.clone())
            .collect()
    }

    /// Clic avec ou sans Ctrl / Maj : ajuste la sélection multiple.
    fn click_selection(&mut self, status: &Status, path: &PathBuf, ctrl: bool, shift: bool) {
        let order = self.ordered_paths(status);
        if shift {
            let anchor = self
                .anchor
                .clone()
                .or_else(|| self.selected.clone())
                .unwrap_or_else(|| path.clone());
            let from = order.iter().position(|p| *p == anchor);
            let to = order.iter().position(|p| p == path);
            if let (Some(a), Some(b)) = (from, to) {
                self.multi = order[a.min(b)..=a.max(b)].iter().cloned().collect();
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

    /// Sélection multiple depuis le mode scénario (`git.multi a,b`).
    pub fn select_many(&mut self, wanted: &[String]) -> bool {
        let Some(status) = self.status.clone() else {
            return false;
        };
        let mut set = HashSet::new();
        for w in wanted {
            let w = w.replace('\\', "/");
            match status.files.iter().find(|f| f.repo_path == w) {
                Some(f) => {
                    set.insert(f.path.clone());
                }
                None => return false,
            }
        }
        let first = status
            .files
            .iter()
            .find(|f| set.contains(&f.path))
            .map(|f| f.path.clone());
        self.multi = set;
        if let Some(first) = first {
            self.anchor = Some(first.clone());
            self.select(first);
        }
        true
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

    pub fn ask_skip(&mut self) -> bool {
        match self.status.as_ref().and_then(|s| s.operation.as_ref()) {
            Some(operation) if operation.kind != OperationKind::Merge => {
                self.confirm = Some(Confirm::SkipOperation(operation.kind));
                true
            }
            _ => false,
        }
    }

    /// « Continuer » (rebase, cherry-pick, revert) : rend l'intention, ou la raison du refus.
    pub fn continue_intent(&self) -> Result<GitIntent, String> {
        let status = self.status.as_ref().ok_or("état Git inconnu")?;
        let operation = status
            .operation
            .as_ref()
            .ok_or("aucune opération en cours")?;
        if operation.kind == OperationKind::Merge {
            return Err("une fusion se conclut par « Valider » (git.commit)".to_owned());
        }
        if status.conflicts().next().is_some() {
            return Err("il reste des fichiers en conflit".to_owned());
        }
        Ok(GitIntent::ContinueOperation(None))
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
            Confirm::SkipOperation(_) => out.push(GitIntent::SkipOperation),
        }
    }

    pub fn dismiss_error(&mut self) {
        self.commit_error = None;
    }

    pub fn select_log(&mut self, index: usize, out: &mut Vec<GitIntent>) -> bool {
        self.log.select(index, out)
    }

    pub fn open_commit_file(&mut self, index: usize, out: &mut Vec<GitIntent>) -> bool {
        let Some((id, file)) = self.log.open_commit_file(index) else {
            return false;
        };
        out.push(GitIntent::RequestDiff {
            request: DiffRequest::commit_file(&id, &file),
            job: DiffJob {
                target: DiffTarget::Window,
                token: 0,
            },
        });
        true
    }

    /// « Historique du fichier » : le journal filtré sur ce chemin.
    pub fn show_file_history(&mut self, path: PathBuf) {
        self.tab = GitTab::Log;
        self.log.set_filters(Filters {
            path: Some(path),
            ..Filters::default()
        });
    }

    /// Le journal montre ces commits (comparaison de branches : `C..X`).
    pub fn show_log_range(&mut self, branch: String) {
        self.tab = GitTab::Log;
        self.log.set_filters(Filters {
            branch: Some(branch),
            ..Filters::default()
        });
    }

    /// Le journal repart de zéro (après une opération qui change l'historique).
    pub fn reset_log_now(&mut self) {
        self.log.reset();
    }

    // ----- Affichage -------------------------------------------------------------

    pub fn show(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        state: RepoState,
        commit_shortcut: &str,
        branches: Option<&BranchList>,
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
        if let Some(text) = self.stash_banner_text() {
            let banner = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 30.0));
            self.show_stash_banner(ui, banner, p, &text, &mut out);
            top += 30.0;
        }
        if let Some(text) = self.detached_banner_text() {
            let banner = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), 30.0));
            self.show_detached_banner(ui, banner, p, &text, &mut out);
            top += 30.0;
        }
        // Barre d'onglets Git.
        let bar = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), TAB_BAR));
        self.show_tab_bar(ui, bar, p, &mut out);
        let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
        match self.tab {
            GitTab::Changes => self.show_changes(ui, body, p, commit_shortcut, &mut out),
            GitTab::Log => self.log.show(ui, body, p, branches, &mut out),
            GitTab::Console => self.show_console(ui, body, p),
        }
        self.show_modals(ui.ctx(), p, &mut out);
        if let Some(request) = self.log.menu_request.take() {
            self.menu_request = Some(request);
        }
        out
    }

    pub fn list_has_focus(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.list_focus))
    }

    /// Bandeau « Vos changements locaux sont mis de côté… » (avec « Remettre » quand rien ne
    /// les remettra seul).
    fn show_stash_banner(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        text: &str,
        out: &mut Vec<GitIntent>,
    ) {
        ui.painter()
            .rect_filled(rect, 0.0, p.warn.gamma_multiply(0.22));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0)),
            Icon::Warning,
            p.warn,
        );
        let main = self.main_pending().cloned();
        let button = main.as_ref().and_then(|m| match m.state {
            PendingStashState::AwaitingOperation(_) => None,
            PendingStashState::OperationOver => Some((
                t::BTN_RESTORE_STASH,
                Some(t::TIP_RESTORE_STASH_OVER),
                GitIntent::RestorePendingStash,
            )),
            PendingStashState::Kept => Some((
                t::BTN_RESTORE_STASH_SHORT,
                None,
                GitIntent::RestoreStash(m.entry.commit.clone()),
            )),
        });
        let text_right = if button.is_some() {
            rect.right() - 190.0
        } else {
            rect.right() - 10.0
        };
        let clip = ui
            .painter()
            .with_clip_rect(Rect::from_min_max(rect.min, pos2(text_right, rect.max.y)));
        clip.text(
            pos2(rect.left() + 34.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.5),
            p.text,
        );
        if let Some((label, tip, intent)) = button {
            kit::child(ui, rect.shrink2(vec2(10.0, 4.0)), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let response = kit::button(ui, p, label, true, true);
                    let response = match tip {
                        Some(tip) => response.on_hover_text(tip),
                        None => response,
                    };
                    if response.clicked() {
                        out.push(intent);
                    }
                });
            });
        }
    }

    /// Bandeau « Vos commits ne sont sur aucune branche » avec « Créer une branche ici ».
    fn show_detached_banner(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        text: &str,
        out: &mut Vec<GitIntent>,
    ) {
        ui.painter()
            .rect_filled(rect, 0.0, p.error.gamma_multiply(0.25));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0)),
            Icon::Warning,
            p.error,
        );
        let clip = ui.painter().with_clip_rect(Rect::from_min_max(
            rect.min,
            pos2(rect.right() - 190.0, rect.max.y),
        ));
        clip.text(
            pos2(rect.left() + 34.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(14.0),
            p.text,
        );
        kit::child(ui, rect.shrink2(vec2(10.0, 4.0)), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::button(ui, p, t::BTN_CREATE_BRANCH_HERE, true, true).clicked() {
                    out.push(GitIntent::NewBranchHere);
                }
            });
        });
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
        let step = status.operation.as_ref().and_then(|o| o.step);
        let text = if conflicts > 0 {
            match step {
                Some(step) if kind == OperationKind::Rebase => t::rebase_conflicts(step, conflicts),
                _ => t::operation_conflicts(kind, conflicts),
            }
        } else if kind == OperationKind::Merge {
            t::OPERATION_RESOLVED.to_owned()
        } else {
            t::operation_resolved_continue(kind)
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
        let rebase_like = kind != OperationKind::Merge;
        kit::child(ui, rect.shrink2(vec2(10.0, 4.0)), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if kit::button(ui, p, t::operation_abort_label(kind), false, true).clicked() {
                    self.ask_abort();
                }
                if rebase_like {
                    if kit::button(ui, p, t::BTN_SKIP_COMMIT, false, true).clicked() {
                        self.ask_skip();
                    }
                    let can = conflicts == 0;
                    let response = kit::button(ui, p, t::BTN_CONTINUE_OP, true, can);
                    if can && response.clicked() {
                        out.push(GitIntent::ContinueOperation(None));
                    } else if !can {
                        response.on_disabled_hover_text(t::TIP_CONFLICTS_REMAIN);
                    }
                } else if conflicts == 0
                    && kit::button(ui, p, t::BTN_VALIDATE_MERGE, true, true).clicked()
                {
                    self.show_changes_for_commit();
                }
                if conflicts > 0 && kit::button(ui, p, t::OPERATION_RESOLVE, true, true).clicked() {
                    self.tab = GitTab::Changes;
                    if let Some(first) = status.conflicts().next() {
                        self.select(first.path.clone());
                    }
                }
            });
        });
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
                self.log.reset();
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
        // Fenêtre basse (bandeaux en plus) : la zone de message cède de la place à la liste.
        let message_height = (rect.height() - 130.0).clamp(84.0, 112.0);
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
        let mut menu_for: Option<(Vec<PathBuf>, egui::Pos2)> = None;
        // Clic droit hors de la sélection : elle devient cette ligne.
        let mut plain = false;
        let order = self.ordered_paths(status);
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
                        if response.secondary_clicked() && group != Group::Conflicted {
                            menu_for = Some((
                                files.iter().map(|f| f.path.clone()).collect(),
                                response
                                    .interact_pointer_pos()
                                    .unwrap_or(header.left_bottom()),
                            ));
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
                                let selected = self.selected.as_ref() == Some(&file.path)
                                    || self.multi.contains(&file.path);
                                let row = file_row(ui, p, file, selected, checked);
                                if row.secondary {
                                    if self.multi.len() > 1 && self.multi.contains(&file.path) {
                                        // Clic droit dans la sélection : le menu agit sur toute la sélection.
                                        let all: Vec<PathBuf> = order
                                            .iter()
                                            .filter(|p| self.multi.contains(*p))
                                            .cloned()
                                            .collect();
                                        menu_for = Some((all, row.pos));
                                    } else {
                                        menu_for = Some((vec![file.path.clone()], row.pos));
                                        plain = true;
                                        clicked = Some(file.path.clone());
                                    }
                                } else if row.toggled {
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
        if let Some((paths, pos)) = menu_for {
            self.menu_request = Some(MenuRequest {
                target: Target::Files {
                    zone: Zone::Changes,
                    files: paths
                        .into_iter()
                        .map(|path| FileRef {
                            path,
                            is_dir: false,
                        })
                        .collect(),
                },
                pos,
                by_keyboard: false,
            });
        }
        for (path, value) in toggled {
            if value {
                self.checked.insert(path);
            } else {
                self.checked.remove(&path);
            }
        }
        if let Some(path) = clicked.or(double.clone()) {
            ui.memory_mut(|m| m.request_focus(self.list_focus));
            let (ctrl, shift) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
            self.click_selection(status, &path, ctrl && !plain, shift && !plain);
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
        let shift_down = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::ArrowDown));
        let shift_up = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::ArrowUp));
        let target = match (position, down || shift_down, up || shift_up) {
            (None, true, _) | (None, _, true) => Some(0),
            (Some(i), true, _) if i + 1 < ordered.len() => Some(i + 1),
            (Some(i), _, true) if i > 0 => Some(i - 1),
            _ => None,
        };
        if let Some(i) = target {
            if let Some(file) = ordered.get(i) {
                let path = file.path.clone();
                if shift_down || shift_up {
                    if self.anchor.is_none() {
                        self.anchor = self.selected.clone();
                    }
                    self.click_selection(status, &path, false, true);
                } else {
                    self.multi.clear();
                    self.anchor = Some(path.clone());
                }
                self.select(path);
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
        let labels = self.labels();
        if let Some(kind) = kind {
            clip.text(
                pos2(rect.right() - 8.0, rect.top() + 14.0),
                Align2::RIGHT_CENTER,
                t::conflict_kind_for(kind, labels.yours()),
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
                // Suppressions : les boutons disent ce qui arrive au fichier, dans l'ordre des
                // étages de Git. Sinon : « votre version » d'abord, puis ce que l'on intègre.
                let deletion = matches!(
                    kind,
                    Some(ConflictKind::DeletedByThem | ConflictKind::DeletedByUs)
                );
                let buttons: [(ConflictSide, String, Option<String>); 2] = if deletion {
                    let (a, b) = t::keep_labels(kind);
                    [
                        (ConflictSide::Ours, a.to_owned(), None),
                        (ConflictSide::Theirs, b.to_owned(), None),
                    ]
                } else {
                    let yours = labels.yours();
                    let incoming = labels.incoming();
                    [
                        (
                            yours,
                            t::KEEP_YOURS.to_owned(),
                            Some(t::keep_yours_tip(labels.side(yours))),
                        ),
                        (
                            incoming,
                            t::keep_incoming(labels.side(incoming)),
                            Some(t::keep_incoming_tip(labels.side(incoming))),
                        ),
                    ]
                };
                for (side, label, tip) in buttons {
                    let response = ui.small_button(label);
                    let response = match tip {
                        Some(tip) => response.on_hover_text(tip),
                        None => response,
                    };
                    if response.clicked() {
                        *keep = Some((side, file.path.clone()));
                    }
                }
                let disabled = self.merge_disabled(file);
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
                if let Some(reason) = &disabled {
                    merge.on_disabled_hover_text(reason.as_str());
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
                    .desired_rows(if rect.height() < 100.0 { 2 } else { 3 })
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
                    let labels = self.labels();
                    let yours = labels.yours() == *side;
                    let label = labels.side(*side);
                    (
                        t::confirm_keep_title_for(yours, label, &names),
                        t::confirm_keep_detail_for(yours, label, &deletes),
                    )
                }
                Confirm::AbortOperation(kind) => (
                    t::confirm_abort_title(*kind),
                    t::confirm_abort_detail(*kind),
                ),
                Confirm::SkipOperation(kind) => (
                    t::confirm_skip_title(*kind),
                    t::CONFIRM_SKIP_DETAIL.to_owned(),
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
                        "merge_check": self.merge_check(&f.path).map(|c| format!("{c:?}")),
                        "merge_disabled": self.merge_disabled(f),
                        "checked": self.checked.contains(&f.path),
                        "selected": self.selected.as_ref() == Some(&f.path) || self.multi.contains(&f.path),
                    })
                })
                .collect::<Vec<_>>()
        });
        serde_json::json!({
            "tab": self.tab.key(),
            "files": files,
            "operation": self.status.as_ref().and_then(|s| s.operation.as_ref()).map(|o| format!("{:?}", o.kind)),
            "conflict_labels": {
                "yours": format!("{:?}", self.labels().yours()),
                "ours": side_json(&self.labels().ours),
                "theirs": side_json(&self.labels().theirs),
                "keep_buttons": self.status.as_ref().map(|s| s.conflicts().map(|f| {
                    let labels = self.labels();
                    serde_json::json!({
                        "file": f.repo_path,
                        "kind": f.conflict.map(|c| t::conflict_kind_for(c, labels.yours())),
                        "yours": t::KEEP_YOURS,
                        "incoming": t::keep_incoming(labels.side(labels.incoming())),
                    })
                }).collect::<Vec<_>>()),
            },
            "pending_stashes": self.pending_stashes.iter().map(|s| serde_json::json!({
                "message": s.message,
                "commit": s.entry.commit,
                "state": match s.state {
                    PendingStashState::AwaitingOperation(k) => format!("AwaitingOperation({k:?})"),
                    PendingStashState::OperationOver => "OperationOver".to_owned(),
                    PendingStashState::Kept => "Kept".to_owned(),
                },
            })).collect::<Vec<_>>(),
            "detached": self.detached.as_ref().map(|d| serde_json::json!({
                "head": d.head, "orphan_commits": d.orphan_commits, "subjects": d.subjects,
            })),
            "banners": {
                "stash": self.stash_banner_text(),
                "stash_button": self.main_pending().and_then(|m| match m.state {
                    PendingStashState::AwaitingOperation(_) => None,
                    PendingStashState::OperationOver => Some(t::BTN_RESTORE_STASH),
                    PendingStashState::Kept => Some(t::BTN_RESTORE_STASH_SHORT),
                }),
                "detached": self.detached_banner_text(),
                "detached_button": self.detached_banner_text().map(|_| t::BTN_CREATE_BRANCH_HERE),
            },
            "message": self.message,
            "amend": self.amend,
            "commit_pending": self.commit_pending,
            "commit_error": self.commit_error,
            "confirm": self.confirm.as_ref().map(|c| match c {
                Confirm::KeepSide { side, files } => format!("keep_{side:?}:{}", files.len()),
                Confirm::AbortOperation(k) => format!("abort:{k:?}"),
                Confirm::SkipOperation(k) => format!("skip:{k:?}"),
            }),
            "preview": self.preview.state(),
            "log": self.log.state(),
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
    secondary: bool,
    pos: egui::Pos2,
}

fn side_json(label: &julie_vcs::SideLabel) -> serde_json::Value {
    serde_json::json!({
        "role": label.role.map(|r| format!("{r:?}")),
        "name": label.name,
        "commit": label.commit,
        "subject": label.subject,
        "title": t::merge_side_title(label),
    })
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
    let kind_rect = clip.text(
        pos2(rect.right() - 8.0, rect.center().y),
        Align2::RIGHT_CENTER,
        t::file_kind(file.kind),
        FontId::proportional(11.5),
        p.text_dim,
    );
    // État d'index : « indexé » ou « en partie indexé » (de quoi retirer de l'index).
    let mark = match file.index_state() {
        IndexState::Staged => Some(t::INDEX_STAGED),
        IndexState::PartiallyStaged => Some(t::INDEX_PARTIAL),
        _ => None,
    };
    if let Some(mark) = mark {
        clip.text(
            pos2(kind_rect.left() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            mark,
            FontId::proportional(11.5),
            p.git_added,
        );
    }
    RowEvent {
        clicked: response.clicked() && !toggled,
        double: response.double_clicked(),
        toggled,
        secondary: response.secondary_clicked(),
        pos: response
            .interact_pointer_pos()
            .unwrap_or(rect.left_bottom()),
    }
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
