//! L'application : état global, boucle d'affichage, distribution des
//! événements du bus et des raccourcis.
//!
//! Règles : aucune règle métier ici (l'interface affiche ce que les crates
//! publient et leur renvoie des intentions) ; rien de bloquant dans `ui` ;
//! redessin uniquement sur événement (sauf pendant un scénario, qui demande ses
//! images, et la relève du disque toutes les 2 s tant qu'un fichier est ouvert).

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Frame, Id, Key, Margin, Modifiers, Sense};
use julie_core::bus::Bus;
use julie_core::file::LineEnding;
use julie_core::settings::{Scope, SettingValue, SettingsProblem};
use julie_core::{
    Domain, DomainEvent, Event, Settings, SettingsFiles, SettingsWatcher, TaskId, Tasks,
    TextPosition,
};
use julie_diff::{ApplyScope, MergeSession, Side};
use julie_index::{IndexEvent, IndexService, IndexSettings};
use julie_phpunit::{PhpUnit, PhpUnitEvent, PhpUnitSettings, RunRequest};
use julie_terminal::{TerminalEvent, TerminalSettings};
use julie_text::{TextError, TextEvent};
use julie_vcs::{
    BranchList, ConflictSide, ConflictVersions, DiffSides, IntegrationMethod, PushRequest, Vcs,
    VcsEvent, VcsSettings,
};

use self::gitflow::{Pending, ToastAction};
use crate::actions::{ActionId, Menu, Registry};
use crate::branchui::{self, BranchPopup};
use crate::ctxmenu::ContextMenu;
use crate::diffpane::{self, DiffPane};
use crate::fsdialog::FsDialog;
use crate::gitdialogs::Dialog as GitDialog;
use crate::gitview::{DiffJob, DiffTarget, GitIntent, GitTab, GitView, MergeCheck, RepoState};
use crate::icons::{self, Icon};
use crate::merge::{self, MergeDialog, MergeIntent};
use crate::nav::NavState;
use crate::project::{self, Channel, GitInfo, Msg, ProjectState};
use crate::safety;
use crate::scenario::{self, Injection, Scenario, Step};
use crate::search::find_in_files::{self, FindInFiles};
use crate::search::goto_file::{self, GotoFile};
use crate::search::palette::{self, ActionPalette};
use crate::settingsui::{Page, SettingsDialog};
use crate::status::{self, StatusModel, TaskTracker};
use crate::tabs::{BannerIntent, Editors, Goto, TabIntent};
use crate::target::Target;
use crate::terminal::{TerminalIntent, TerminalPanel};
use crate::testsview::{TestsIntent, TestsView};
use crate::texts as t;
use crate::theme::{self, Palette};
use crate::tools::{self, BottomTool, ToolWindows};
use crate::tree::ProjectTree;
use crate::usages::UsagesView;

mod fsops;
mod gitflow;
mod menus;
mod navigation;
mod scenario_ui;
mod settings;

pub struct Options {
    pub scenario: Option<PathBuf>,
    pub paths: Vec<PathBuf>,
}

enum Dialog {
    /// Fermer un onglet modifié.
    CloseTab(PathBuf),
    /// Quitter avec des onglets modifiés.
    Quit(Vec<PathBuf>),
    /// Fermer plusieurs onglets dont certains sont modifiés (une seule question).
    CloseMany(Vec<PathBuf>),
}

struct Message {
    text: String,
    error: bool,
    at: Instant,
    /// Bouton de la notification (« Restaurer »).
    action: Option<ToastAction>,
}

impl Message {
    /// Une information s'efface vite ; une erreur reste le temps de la lire.
    fn lifetime(&self) -> Duration {
        Duration::from_secs(if self.error { 12 } else { 6 })
    }
}

/// Ce que publient les tâches de calcul de l'application (domaine `Diff`).
enum AppDiffEvent {
    /// Un diff calculé, pour l'aperçu ou la grande fenêtre.
    Prepared {
        job: DiffJob,
        prepared: Box<diffpane::Prepared>,
    },
    /// La session de fusion est prête.
    MergeReady {
        versions: Box<ConflictVersions>,
        session: Box<MergeSession>,
    },
    /// Résultat de la vérification d'avance de « Fusionner… » pour un fichier.
    MergeChecked {
        path: PathBuf,
        result: Result<(), merge::Ineligible>,
    },
}

/// Ce qu'une tâche de diff doit calculer.
enum PrepareInput {
    Sides(Box<DiffSides>, String),
    Texts {
        title: String,
        left_title: String,
        right_title: String,
        left: String,
        right: String,
    },
}

/// La grande fenêtre de diff (changements locaux, fichier d'un commit,
/// attendu / obtenu).
struct DiffWindow {
    pane: DiffPane,
}

pub struct JulieStormApp {
    ctx: egui::Context,
    bus: Bus,
    tasks: Tasks,
    waker: julie_text::Waker,
    channel: Channel,
    rx: Receiver<Msg>,
    dark: bool,
    registry: Registry,
    project: Option<ProjectState>,
    /// Nombre d'ouvertures de projet en cours.
    opening: u32,
    scan_task: Option<TaskId>,
    editors: Editors,
    tree: ProjectTree,
    tools: ToolWindows,
    terminal: Option<TerminalPanel>,
    goto: Option<GotoFile>,
    find: FindInFiles,
    find_open: bool,
    palette: Option<ActionPalette>,
    tracker: TaskTracker,
    dialog: Option<Dialog>,
    message: Option<Message>,
    last_title: String,
    window_focused: bool,
    /// Fichiers que la personne vient de demander d'ouvrir : leur ouverture réussie
    /// efface une erreur de la barre d'état.
    user_opens: HashSet<PathBuf>,
    last_disk_poll: Instant,
    force_close: bool,
    quit_after_save: bool,
    scenario: Option<Scenario>,
    scenario_active: bool,
    injected: VecDeque<Injection>,
    remember_last: bool,
    git: GitView,
    tests: TestsView,
    merge: Option<MergeDialog>,
    diff_window: Option<DiffWindow>,
    last_status_poll: Instant,
    // ----- v1 : branches, menus contextuels, dialogues Git -----------------------
    branches: Option<BranchList>,
    /// Empreinte des branches et de leurs sommets : le journal est relu quand elle change.
    branch_sig: Vec<(String, String)>,
    branch_popup: Option<BranchPopup>,
    /// Rectangle du widget de branche (ancre de la fenêtre surgissante).
    branch_widget: Option<egui::Rect>,
    ctx_menu: Option<ContextMenu>,
    ctx_target: Option<Target>,
    git_dialog: Option<GitDialog>,
    pending_switch: Option<Pending>,
    /// Un push à rejouer après « Fusionner / Rebaser puis pousser ».
    after_update_push: Option<PushRequest>,
    last_push: Option<PushRequest>,
    /// Suppression de la branche distante demandée avec celle de la locale.
    delete_remote_after: Option<(String, String)>,
    /// Méthode de mise à jour mémorisée **pour la session** (le noyau ne sait pas
    /// encore écrire les réglages).
    update_method: Option<IntegrationMethod>,
    net_progress: Option<String>,
    last_fetch: Option<Instant>,
    /// Dossier de la dernière copie de sécurité (« Restaurer »).
    last_copy: Option<String>,
    // ----- v1 bis : réglages -----------------------------------------------------
    /// Fichier des réglages « Tous mes projets » (`None` : pas de dossier de configuration).
    user_settings_file: Option<PathBuf>,
    /// Les réglages effectifs en vigueur (derniers valides).
    settings: Settings,
    settings_files: SettingsFiles,
    settings_watcher: Option<SettingsWatcher>,
    /// Bus du surveillant courant (un par projet ouvert).
    settings_bus: Bus,
    /// Le prochain événement du surveillant est le premier (réglages initiaux).
    settings_first: bool,
    settings_problems: Vec<SettingsProblem>,
    settings_dialog: Option<SettingsDialog>,
    /// Dernière page ouverte, retenue pour la session.
    settings_page: Option<Page>,
    // ----- v1 bis : arbre du projet ----------------------------------------------
    fs_dialog: Option<FsDialog>,
    /// Renommage envoyé à `git mv` : de quoi retomber sur un renommage simple (`NotTracked`).
    pending_rename: Option<(PathBuf, PathBuf)>,
    /// « Supprimer aussi les fichiers ajoutés » était cochée à l'annulation en cours.
    discard_delete_added: bool,
    /// Menu à ouvrir quelques images plus tard (le temps qu'un clic droit place le curseur).
    deferred_menu: Option<(u8, Target)>,
    deferred_click_pos: egui::Pos2,
    // ----- v1 : navigation dans le code ------------------------------------------
    nav: NavState,
    usages: UsagesView,
}

impl JulieStormApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> Self {
        let ctx = cc.egui_ctx.clone();
        let bus = Bus::new();
        // egui ne redessine que sur demande : chaque publication d'un fil de fond
        // réclame une image. `request_repaint` est sûr depuis n'importe quel fil.
        let waker_ctx = ctx.clone();
        bus.set_waker(move || waker_ctx.request_repaint());
        let waker_ctx = ctx.clone();
        let waker: julie_text::Waker = Arc::new(move || waker_ctx.request_repaint());
        let tasks = Tasks::new(bus.publisher());
        let (tx, rx) = channel();
        let channel = Channel {
            tx,
            ctx: ctx.clone(),
        };
        let dark = true;
        theme::apply(&ctx, dark);

        let mut app = Self {
            ctx: ctx.clone(),
            bus,
            tasks,
            waker,
            channel,
            rx,
            dark,
            registry: Registry::new(),
            project: None,
            opening: 0,
            scan_task: None,
            editors: Editors::default(),
            tree: ProjectTree::new(),
            tools: ToolWindows::default(),
            terminal: None,
            goto: None,
            find: FindInFiles::new(),
            find_open: false,
            palette: None,
            tracker: TaskTracker::default(),
            dialog: None,
            message: None,
            last_title: String::new(),
            window_focused: true,
            user_opens: HashSet::new(),
            last_disk_poll: Instant::now(),
            force_close: false,
            quit_after_save: false,
            scenario: None,
            scenario_active: false,
            injected: VecDeque::new(),
            remember_last: options.scenario.is_none() || project::state_dir_overridden(),
            git: GitView::new(dark),
            tests: TestsView::new(PathBuf::new(), None),
            merge: None,
            diff_window: None,
            last_status_poll: Instant::now(),
            branches: None,
            branch_sig: Vec::new(),
            branch_popup: None,
            branch_widget: None,
            ctx_menu: None,
            ctx_target: None,
            git_dialog: None,
            pending_switch: None,
            after_update_push: None,
            last_push: None,
            delete_remote_after: None,
            update_method: None,
            net_progress: None,
            last_fetch: None,
            last_copy: None,
            user_settings_file: project::user_settings_file(options.scenario.is_some()),
            settings: Settings::default(),
            settings_files: SettingsFiles::new(None, None),
            settings_watcher: None,
            settings_bus: Bus::new(),
            settings_first: true,
            settings_problems: Vec::new(),
            settings_dialog: None,
            settings_page: None,
            fs_dialog: None,
            pending_rename: None,
            discard_delete_added: false,
            deferred_menu: None,
            deferred_click_pos: egui::Pos2::ZERO,
            nav: NavState::default(),
            usages: UsagesView::default(),
        };
        app.restart_settings_watcher(None);

        if let Some(file) = &options.scenario {
            app.load_scenario(file);
        }
        // Premier dossier en argument : le projet. Les fichiers : onglets.
        let mut project_arg = None;
        for path in &options.paths {
            if path.is_dir() {
                project_arg.get_or_insert_with(|| path.clone());
            } else {
                app.editors.open(&app.tasks, path.clone(), None);
            }
        }
        match project_arg {
            Some(dir) => app.start_open(dir),
            None if app.remember_last => project::spawn_read_last(app.channel.clone()),
            None => {}
        }
        app
    }

    fn palette(&self) -> Palette {
        theme::palette(self.dark)
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        // Une seule ligne dans la barre d'état ; le détail est dans la Console Git.
        let text: String = text.into();
        let mut text = text.lines().next().unwrap_or_default().to_owned();
        if text.chars().count() > 160 {
            text = text.chars().take(157).collect::<String>() + "…";
        }
        self.message = Some(Message {
            text,
            error,
            at: Instant::now(),
            action: None,
        });
    }

    /// Une action réussie efface l'erreur affichée dans la barre d'état.
    fn clear_error_message(&mut self) {
        if self.message.as_ref().is_some_and(|m| m.error) {
            self.message = None;
        }
    }

    // ----- Projet --------------------------------------------------------------

    fn start_open(&mut self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        self.opening += 1;
        project::spawn_open(path, self.user_settings_file.clone(), self.channel.clone());
    }

    fn pump_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::ProjectReady { project, settings } => {
                    self.opening = self.opening.saturating_sub(1);
                    self.on_project_ready(project, settings);
                }
                Msg::ProjectFailed { path, error } => {
                    self.opening = self.opening.saturating_sub(1);
                    self.say(t::project_open_failed(&path, &error), true);
                }
                Msg::FolderPicked(Some(path)) => self.start_open(path),
                Msg::FolderPicked(None) => {}
                Msg::LastProject(Some(path)) => {
                    if self.project.is_none() && self.opening == 0 {
                        self.start_open(path);
                    }
                }
                Msg::LastProject(None) => {}
                Msg::DirListed { dir, entries } => self.tree.set_listing(dir, entries),
                Msg::SafetyCopied { then, result } => self.on_safety_copied(then, result),
                Msg::Notice { text, error } => self.say(text, error),
                Msg::SettingsApplied { close, result } => self.on_settings_applied(close, result),
                Msg::SettingsWritten { result } => self.on_settings_written(result),
                Msg::SettingsProbe {
                    field,
                    token,
                    state,
                } => {
                    if let Some(dialog) = &mut self.settings_dialog {
                        dialog.probe_result(field, token, state);
                    }
                }
                Msg::SettingsBrowsed { field, scope, path } => {
                    let root = self.project_root();
                    if let Some(dialog) = &mut self.settings_dialog {
                        dialog.browsed(field, scope, &path, root.as_deref());
                    }
                }
                Msg::OpenPath { path, line } => {
                    let goto = line.map(|line| Goto {
                        position: TextPosition::new(line.saturating_sub(1), 0),
                        end: None,
                    });
                    self.open_file(path, goto);
                }
                Msg::Trashed { paths, result } => self.on_trashed(paths, result),
                Msg::Moved { from, to } => self.after_move(&from, &to),
                Msg::FilesChanged {
                    text,
                    error,
                    refresh,
                    reveal,
                } => {
                    if let Some(path) = reveal {
                        self.tree.expand_path(&path);
                    }
                    if refresh {
                        self.tree.refresh();
                        if let Some(project) = &self.project {
                            if let Some(repo) = &project.repo {
                                project.vcs.refresh_status(repo);
                            }
                        }
                    }
                    if let Some(text) = text {
                        self.say(text, error);
                    }
                }
            }
        }
    }

    fn on_project_ready(
        &mut self,
        project: julie_core::Project,
        settings: Result<Settings, String>,
    ) {
        // Un autre projet : les messages de l'ancien n'ont plus de sens.
        self.message = None;
        self.user_opens.clear();
        let settings = match settings {
            Ok(settings) => settings,
            Err(error) => {
                self.say(t::settings_failed(&error), true);
                Settings::default()
            }
        };
        self.settings = settings.clone();
        self.restart_settings_watcher(Some(&project));
        let dark = settings
            .get(&["app", "theme"])
            .and_then(|v| v.as_str())
            .is_none_or(|v| v != "clair");
        if dark != self.dark {
            self.set_dark(dark);
        }
        let mut index_settings: IndexSettings = settings.section("index").unwrap_or_default();
        for always in julie_core::project::ALWAYS_EXCLUDED.iter().rev() {
            if !index_settings.excluded.iter().any(|e| e == always) {
                index_settings.excluded.insert(0, (*always).to_owned());
            }
        }
        let vcs_settings: VcsSettings = settings.section("vcs").unwrap_or_default();
        let terminal_settings: TerminalSettings = settings.section("terminal").unwrap_or_default();
        let phpunit_settings = PhpUnitSettings::from_settings(&settings).unwrap_or_default();

        if let Some(old) = &self.project {
            old.index.cancel_search();
        }
        self.nav_reset_for_project();
        if let Some(id) = self.scan_task.take() {
            self.tasks.cancel(id);
        }
        let root = project.root().to_path_buf();
        let index = Arc::new(IndexService::new(
            self.tasks.clone(),
            project.clone(),
            index_settings,
        ));
        self.scan_task = Some(index.scan().id());
        let vcs = Vcs::new(self.tasks.clone(), &vcs_settings);
        vcs.open(root.clone());
        self.tree
            .reset(root.clone(), index.clone(), self.channel.clone());
        // Les terminaux repartent dans la racine du nouveau projet.
        self.terminal = Some(TerminalPanel::new(
            self.tasks.clone(),
            root.clone(),
            terminal_settings,
        ));
        self.git = GitView::new(self.dark);
        self.branches = None;
        self.branch_sig.clear();
        self.branch_popup = None;
        self.ctx_menu = None;
        self.git_dialog = None;
        self.pending_switch = None;
        self.after_update_push = None;
        self.update_method = vcs_settings.update_method;
        {
            let root = root.clone();
            std::thread::spawn(move || safety::purge(&root));
        }
        self.merge = None;
        self.diff_window = None;
        self.tests = TestsView::new(
            root.clone(),
            Some(PhpUnit::new(
                self.tasks.clone(),
                project.clone(),
                phpunit_settings,
            )),
        );
        self.project = Some(ProjectState {
            project,
            index,
            vcs,
            repo: None,
            git: GitInfo::Unknown,
            index_ready: false,
        });
        self.goto = None;
        self.find = FindInFiles::new();
        self.find_open = false;
        self.palette = None;
        if self.remember_last {
            project::spawn_write_last(root);
        }
        self.tools.project_open = true;
        self.ctx.request_repaint();
    }

    // ----- Événements des crates -----------------------------------------------

    fn pump_bus(&mut self) {
        for event in self.bus.drain() {
            match event {
                Event::Task(task) => self.tracker.on_event(&task),
                Event::Domain(event) => self.on_domain(event),
                // Les réglages changés (`SettingsChanged`, ajout du noyau) n'ont pas encore de
                // consommateur dans l'interface : la fenêtre de réglages viendra.
                Event::ProjectOpened(_)
                | Event::FilesChangedOnDisk(_)
                | Event::SettingsChanged(_) => {}
            }
        }
    }

    fn on_domain(&mut self, event: DomainEvent) {
        let task = event.task;
        match event.domain {
            Domain::Text => {
                if let Ok(event) = event.take::<TextEvent>() {
                    self.on_text(event);
                }
            }
            Domain::Terminal => {
                if let (Some(terminal), Some(event)) =
                    (&mut self.terminal, event.get::<TerminalEvent>())
                {
                    terminal.on_event(event);
                }
            }
            Domain::Index => {
                if let Ok(event) = event.take::<IndexEvent>() {
                    self.on_index(task, event);
                }
            }
            Domain::Vcs => {
                if let Some(event) = event.get::<VcsEvent>() {
                    self.on_vcs(task, event);
                }
            }
            Domain::PhpUnit => {
                if let Some(probe) = event.get::<julie_phpunit::ProbeEvent>() {
                    self.on_probe_event(task, probe);
                } else if let Some(event) = event.get::<PhpUnitEvent>() {
                    self.tests.on_event(task, event);
                    // Fin d'exécution : notification si la fenêtre des tests est cachée.
                    if let PhpUnitEvent::RunFinished(summary) = event {
                        let visible =
                            self.tools.bottom_open && self.tools.bottom == BottomTool::Tests;
                        if !visible && task == self.tests.run_task {
                            self.say(
                                t::tests_summary(summary, summary.duration),
                                !summary.is_green(),
                            );
                        }
                    }
                }
            }
            Domain::Diff => {
                if let Ok(event) = event.take::<AppDiffEvent>() {
                    self.on_diff_event(event);
                }
            }
        }
    }

    fn on_diff_event(&mut self, event: AppDiffEvent) {
        match event {
            AppDiffEvent::Prepared { job, prepared } => match job.target {
                DiffTarget::Preview => {
                    if self.git.accepts(job) {
                        self.git.set_preview(*prepared, &self.waker);
                    }
                }
                DiffTarget::Window => {
                    let mut pane = DiffPane::new("diff-fenetre", self.dark);
                    pane.set(*prepared, &self.waker);
                    self.diff_window = Some(DiffWindow { pane });
                }
            },
            AppDiffEvent::MergeChecked { path, result } => {
                let check = match result {
                    Ok(()) => MergeCheck::Fine,
                    Err(why) => MergeCheck::Refused(why),
                };
                self.git.set_merge_check(&path, check);
                self.run_prechecks();
            }
            AppDiffEvent::MergeReady { versions, session } => {
                self.git.merge_loading = None;
                let Some(project) = &self.project else { return };
                let (our_branch, their_branch) = match &self.git.status() {
                    Some(status) => (
                        project.branch_name(),
                        status
                            .operation
                            .as_ref()
                            .and_then(|o| o.message.as_deref())
                            .and_then(merge::their_name),
                    ),
                    None => (None, None),
                };
                self.merge = Some(MergeDialog::new(
                    &versions,
                    *session,
                    project.project.root(),
                    our_branch,
                    their_branch,
                    &self.waker,
                    self.dark,
                ));
            }
        }
    }

    /// Lance le calcul d'un diff dans une tâche de fond.
    fn spawn_prepare(&mut self, job: DiffJob, input: PrepareInput) {
        self.tasks.spawn(Domain::Diff, "diff.compute", move |ctx| {
            let prepared = match input {
                PrepareInput::Sides(sides, title) => {
                    diffpane::prepare_sides(&sides, title, ctx.cancel_token())
                }
                PrepareInput::Texts {
                    title,
                    left_title,
                    right_title,
                    left,
                    right,
                } => diffpane::prepare_texts(
                    title,
                    left_title,
                    right_title,
                    left,
                    right,
                    ctx.cancel_token(),
                ),
            };
            ctx.publish(AppDiffEvent::Prepared {
                job,
                prepared: Box::new(prepared),
            });
            Ok(())
        });
    }

    fn spawn_merge(&mut self, versions: ConflictVersions, inputs: merge::Inputs) {
        self.tasks.spawn(Domain::Diff, "diff.merge", move |ctx| {
            if let Some(session) = merge::build_session(&inputs, ctx.cancel_token()) {
                ctx.publish(AppDiffEvent::MergeReady {
                    versions: Box::new(versions),
                    session: Box::new(session),
                });
            }
            Ok(())
        });
    }

    fn on_text(&mut self, event: TextEvent) {
        match event {
            TextEvent::Loaded(loaded) => {
                if self.user_opens.remove(&loaded.path) {
                    self.clear_error_message();
                }
                let reloaded = loaded.path.clone();
                let was_open = self.editors.index_of(&reloaded).is_some();
                self.editors.on_loaded(loaded, &self.waker, self.dark);
                // Un onglet rechargé : le fichier a changé hors de l'IDE, l'index le relit.
                if let (true, Some(project)) = (was_open, &self.project) {
                    project.index.update(vec![reloaded]);
                }
            }
            TextEvent::LoadFailed { path, error } => {
                self.editors.on_load_failed(&path);
                let reason = match &error {
                    TextError::Binary { .. } => t::ERR_BINARY.to_owned(),
                    TextError::TooLarge { .. } => t::ERR_TOO_LARGE.to_owned(),
                    other => other.to_string(),
                };
                self.user_opens.remove(&path);
                self.say(t::load_failed(&path, &reason), true);
            }
            TextEvent::Saved(outcome) => {
                self.clear_error_message();
                self.editors.on_saved(&outcome);
                self.git.invalidate_diff();
                if let Some(project) = &self.project {
                    if let Some(repo) = &project.repo {
                        project.vcs.refresh_status(repo);
                    }
                }
                if self.quit_after_save && !self.editors.any_modified() {
                    self.force_close = true;
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            TextEvent::SaveFailed { path, error } => {
                self.quit_after_save = false;
                self.editors.close_after_save.remove(&path);
                self.say(t::save_failed(&path, &error.to_string()), true);
            }
            TextEvent::DiskChecked(states) => {
                let changed = states
                    .iter()
                    .any(|(_, state)| *state != julie_text::DiskState::Unchanged);
                self.editors.on_disk_checked(states, &self.tasks);
                if changed {
                    // Un fichier ouvert a changé dehors : Git le saura tout de suite.
                    self.git.invalidate_diff();
                    self.refresh_git();
                }
            }
        }
    }

    fn on_index(&mut self, task: Option<TaskId>, event: IndexEvent) {
        match event {
            IndexEvent::ScanFinished(_) => {
                if task == self.scan_task {
                    if let Some(project) = &mut self.project {
                        project.index_ready = true;
                    }
                    // La liste de l'arbre peut avoir été faite avant la fin : on la
                    // refait pour marquer les fichiers ignorés.
                    self.tree.refresh();
                }
            }
            IndexEvent::Updated(_) => {}
            IndexEvent::Files(results) => {
                if let Some(goto) = &mut self.goto {
                    goto.on_results(task, results);
                }
            }
            IndexEvent::TextMatches(batch) => self.find.on_matches(task, batch),
            IndexEvent::TextSearchFinished(summary) => self.find.on_finished(task, summary),
            IndexEvent::Preview(preview) => self.find.on_preview(task, preview),
            // Index des symboles : la progression passe par la barre d'état (tâche de
            // fond) ; les réponses de navigation et d'usages ignorent celles de
            // demandes périmées (`task`).
            IndexEvent::SymbolsReady(_) | IndexEvent::SymbolsUpdated { .. } => {
                self.ctx.request_repaint();
            }
            IndexEvent::Navigation(result) => self.on_navigation(task, result),
            IndexEvent::Usages(batch) => self.on_usages(task, batch),
            IndexEvent::UsagesFinished(summary) => self.on_usages_finished(task, summary),
        }
    }

    /// Lance, en tâche de fond, le chargement des versions des fichiers en conflit
    /// pour savoir d'avance si « Fusionner… » est possible.
    fn run_prechecks(&mut self) {
        let wanted = self.git.next_prechecks();
        let Some((vcs, repo)) = self
            .project
            .as_ref()
            .and_then(|p| p.repo.clone().map(|r| (p.vcs.clone(), r)))
        else {
            return;
        };
        for path in wanted {
            let handle = vcs.load_conflict(&repo, path.clone());
            self.git.note_precheck_task(handle.id(), path);
        }
    }

    /// Les versions d'une vérification d'avance sont arrivées (ou ont échoué).
    /// Rend `true` si l'événement lui appartenait.
    fn on_precheck_event(&mut self, task: Option<TaskId>, event: &VcsEvent) -> bool {
        let Some(path) = self.git.precheck_path(task) else {
            return false;
        };
        match event {
            VcsEvent::Conflict(versions) => {
                self.git.finish_precheck(task);
                let versions = versions.clone();
                self.tasks
                    .spawn(Domain::Diff, "diff.merge_check", move |ctx| {
                        ctx.publish(AppDiffEvent::MergeChecked {
                            path,
                            result: merge::eligibility(&versions),
                        });
                        Ok(())
                    });
            }
            VcsEvent::Failed(failure) if failure.action == julie_vcs::Action::Conflict => {
                self.git.finish_precheck(task);
                self.git.set_merge_check(&path, MergeCheck::Unknown);
                self.run_prechecks();
            }
            // Les autres événements de la tâche (journal de commandes…) passent normalement.
            _ => return false,
        }
        true
    }

    fn on_vcs(&mut self, task: Option<TaskId>, event: &VcsEvent) {
        if self.on_precheck_event(task, event) {
            return;
        }
        let mut intents = Vec::new();
        self.git.on_vcs(event, &mut intents);
        match event {
            VcsEvent::Opened(repo) => {
                if let Some(project) = &mut self.project {
                    project.repo = Some(repo.clone());
                }
            }
            VcsEvent::NoRepository { .. } => {
                if let Some(project) = &mut self.project {
                    project.git = GitInfo::NoRepository;
                }
                self.tree.set_git(None);
            }
            VcsEvent::Status(status) => {
                self.tree.set_git(Some(status));
                self.git.on_status(status);
                if let Some(project) = &mut self.project {
                    project.git = GitInfo::Repository(Box::new(status.clone()));
                }
                self.run_prechecks();
            }
            VcsEvent::Diff(sides) => {
                if let Some(job) = self.git.take_job(task) {
                    let root = self
                        .project
                        .as_ref()
                        .map(|p| p.project.root().to_path_buf());
                    let title = diffpane::request_title(&sides.request, root.as_deref());
                    self.spawn_prepare(job, PrepareInput::Sides(Box::new(sides.clone()), title));
                }
            }
            VcsEvent::Conflict(versions) => match merge::inputs(versions) {
                Ok(inputs) => self.spawn_merge(versions.clone(), inputs),
                Err(reason) => {
                    self.git.merge_loading = None;
                    let name = file_name(&versions.path);
                    let reason = reason.reason();
                    self.say(t::merge_refused(reason, &name), true);
                }
            },
            VcsEvent::Resolved { path } => {
                if self.merge.as_ref().is_some_and(|m| &m.path == path) {
                    self.merge = None;
                    self.focus_editor();
                }
            }
            VcsEvent::Failed(failure) => {
                if failure.action == julie_vcs::Action::Resolve {
                    if let Some(dialog) = &mut self.merge {
                        dialog.on_save_failed(&failure.error);
                    }
                }
                if self.v1_failure(failure) {
                    self.run_git_intents(intents);
                    return;
                }
                let text = match failure.action {
                    julie_vcs::Action::Commit | julie_vcs::Action::Continue => {
                        t::COMMIT_FAILED.to_owned()
                    }
                    _ => t::git_failed(&failure.error.to_string()),
                };
                self.say(text, true);
            }
            _ => {}
        }
        self.on_vcs_v1(event);
        self.run_git_intents(intents);
    }

    fn repo_state(&self) -> RepoState {
        match self.project.as_ref().map(|p| &p.git) {
            Some(GitInfo::Repository(_)) => RepoState::Ready,
            Some(GitInfo::NoRepository) => RepoState::NoRepository,
            _ => RepoState::Unknown,
        }
    }

    /// Exécute ce que la vue Git a demandé.
    fn run_git_intents(&mut self, intents: Vec<GitIntent>) {
        if intents.is_empty() {
            return;
        }
        let Some((vcs, repo)) = self
            .project
            .as_ref()
            .and_then(|p| p.repo.clone().map(|r| (p.vcs.clone(), r)))
        else {
            return;
        };
        for intent in intents {
            match intent {
                GitIntent::Refresh => {
                    vcs.refresh_status(&repo);
                }
                GitIntent::RequestDiff { request, job } => {
                    let handle = vcs.load_diff(&repo, request);
                    self.git.note_diff_task(handle.id(), job);
                }
                GitIntent::OpenFile(path) => self.open_file(path, None),
                GitIntent::Commit(request) => {
                    vcs.commit(&repo, request);
                }
                GitIntent::ContinueOperation(message) => {
                    vcs.continue_operation(&repo, message);
                }
                GitIntent::AbortOperation => {
                    vcs.abort_operation(&repo);
                }
                GitIntent::SkipOperation => {
                    vcs.skip_operation(&repo);
                }
                GitIntent::AcceptSide { paths, side } => {
                    vcs.accept_side(&repo, paths, side);
                }
                GitIntent::OpenMerge(path) => {
                    vcs.load_conflict(&repo, path);
                }
                GitIntent::LoadLog(query) => {
                    vcs.load_log(&repo, query);
                }
                GitIntent::LoadCommit(id) => {
                    vcs.load_commit(&repo, id);
                }
                GitIntent::Notify(text) => self.say(text, false),
            }
        }
    }

    fn run_merge_intents(&mut self, intents: Vec<MergeIntent>) {
        for intent in intents {
            match intent {
                MergeIntent::Close => {
                    self.merge = None;
                    self.focus_editor();
                }
                MergeIntent::Save { bytes, expected } => {
                    let Some((vcs, repo)) = self
                        .project
                        .as_ref()
                        .and_then(|p| p.repo.clone().map(|r| (p.vcs.clone(), r)))
                    else {
                        continue;
                    };
                    if let Some(dialog) = &self.merge {
                        vcs.resolve_conflict(&repo, dialog.path.clone(), bytes, expected);
                    }
                }
            }
        }
    }

    fn run_tests_intents(&mut self, intents: Vec<TestsIntent>) {
        for intent in intents {
            match intent {
                TestsIntent::Run => self.start_run(RunRequest::all(), false),
                TestsIntent::Restart => self.run_tests(true),
                TestsIntent::Stop => self.stop_tests(),
                TestsIntent::Open(location) => {
                    let goto = Goto {
                        position: location.position,
                        end: None,
                    };
                    self.open_file(location.path, Some(goto));
                }
                TestsIntent::Compare {
                    title,
                    expected,
                    actual,
                } => {
                    let job = DiffJob {
                        target: DiffTarget::Window,
                        token: 0,
                    };
                    self.spawn_prepare(
                        job,
                        PrepareInput::Texts {
                            title,
                            left_title: t::COMPARE_EXPECTED.to_owned(),
                            right_title: t::COMPARE_ACTUAL.to_owned(),
                            left: expected,
                            right: actual,
                        },
                    );
                }
            }
        }
    }

    /// Relance après confirmation : la demande mise en attente, à défaut la suite entière.
    fn run_tests(&mut self, restart: bool) {
        let request = self
            .tests
            .pending_run
            .take()
            .unwrap_or_else(RunRequest::all);
        self.start_run(request, restart);
    }

    /// Lance ces tests (la suite, un fichier, un nœud, les échecs). Une exécution en cours
    /// demande d'abord confirmation.
    pub(super) fn start_run(&mut self, request: RunRequest, restart: bool) {
        let Some(service) = self.tests.service.clone() else {
            return;
        };
        self.tools.bottom = BottomTool::Tests;
        self.tools.bottom_open = true;
        if self.tests.is_running() {
            if !restart {
                self.tests.pending_run = Some(request);
                self.tests.confirm_restart = true;
                return;
            }
            self.stop_tests();
        }
        self.tests.pending_run = None;
        self.tests.last_request = Some(request.clone());
        let handle = service.run(request);
        self.tests.begin(handle.id());
    }

    fn stop_tests(&mut self) {
        if let (Some(service), Some(task)) = (&self.tests.service, self.tests.run_task) {
            service.stop(task);
        }
    }

    // ----- Actions -------------------------------------------------------------

    fn action_enabled(&self, id: ActionId) -> bool {
        match id {
            ActionId::SaveAll => self.editors.any_modified(),
            ActionId::CloseTab | ActionId::LastTab => !self.editors.items.is_empty(),
            ActionId::NextTab | ActionId::PreviousTab => self.editors.items.len() > 1,
            ActionId::GotoFile | ActionId::FindInFiles => self.project.is_some(),
            ActionId::ReloadFromDisk => self.project.is_some() || !self.editors.items.is_empty(),
            ActionId::GitCommit | ActionId::ToggleCommit | ActionId::GitRefresh => {
                self.repo_state() == RepoState::Ready
            }
            ActionId::NavBack => self.nav.history.can_back(),
            ActionId::NavForward => self.nav.history.can_forward(),
            ActionId::GotoDeclaration | ActionId::FindUsages => self.nav_available(),
            ActionId::UsageNext | ActionId::UsagePrevious => self
                .usages
                .active_tab()
                .is_some_and(|tab| !tab.usages.is_empty()),
            ActionId::ToggleUsages => !self.usages.is_empty(),
            ActionId::TestsRun => self.tests.service.is_some(),
            ActionId::TestsStop => self.tests.is_running(),
            ActionId::TestsRerun => self.tests.service.is_some() && self.tests.has_run(),
            ActionId::OpenFolder
            | ActionId::Quit
            | ActionId::ActionPalette
            | ActionId::ToggleProject
            | ActionId::ToggleGit
            | ActionId::ToggleTests
            | ActionId::ToggleSearch
            | ActionId::ToggleTerminal
            | ActionId::ToggleTheme
            | ActionId::Settings
            | ActionId::BottomTall => true,
            other => self.v1_action_enabled(other),
        }
    }

    pub fn run_action(&mut self, id: ActionId) {
        let ctx = self.ctx.clone();
        match id {
            ActionId::OpenFolder => {
                let start = self
                    .project
                    .as_ref()
                    .map(|p| p.project.root().to_path_buf());
                project::spawn_pick_folder(start, self.channel.clone());
            }
            ActionId::SaveAll => {
                for error in self.editors.save_all(&self.tasks) {
                    self.say(error, true);
                }
            }
            ActionId::CloseTab => {
                if !self.editors.items.is_empty() {
                    self.request_close(self.editors.active);
                }
            }
            ActionId::ReloadFromDisk => {
                self.tree.refresh();
                self.editors.check_disk(&self.tasks);
                if let Some(project) = &mut self.project {
                    project.index_ready = false;
                    self.scan_task = Some(project.index.scan().id());
                    if let Some(repo) = &project.repo {
                        project.vcs.refresh_status(repo);
                    }
                }
            }
            ActionId::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            ActionId::GotoFile => {
                if self.project.is_some() {
                    self.palette = None;
                    self.find_open = false;
                    self.goto = match self.goto {
                        Some(_) => None,
                        None => Some(GotoFile::new()),
                    };
                    if self.goto.is_none() {
                        self.focus_editor();
                    }
                }
            }
            ActionId::FindInFiles => {
                if self.project.is_some() {
                    self.goto = None;
                    self.palette = None;
                    if self.find_open {
                        self.find_open = false;
                        self.focus_editor();
                    } else {
                        self.find_open = true;
                        let prefill = self.selection_prefill();
                        let dir = self.tree.selected_dir();
                        self.find.prepare(prefill, dir);
                        self.find.refresh_now();
                    }
                }
            }
            ActionId::ActionPalette => {
                self.goto = None;
                self.find_open = false;
                self.palette = match self.palette {
                    Some(_) => None,
                    None => Some(ActionPalette::new()),
                };
            }
            ActionId::PreviousTab => self.editors.activate_relative(-1),
            ActionId::NextTab => self.editors.activate_relative(1),
            ActionId::LastTab => self.editors.activate_previous(),
            ActionId::ToggleProject => {
                if self.tools.project_open && self.tree.has_focus(&ctx) {
                    self.tools.project_open = false;
                    self.focus_editor();
                } else if self.tools.project_open && !self.tree.has_focus(&ctx) {
                    self.tree.request_focus();
                } else {
                    self.tools.project_open = true;
                    self.tree.request_focus();
                }
            }
            ActionId::ToggleGit => self.toggle_bottom(BottomTool::Git),
            ActionId::ToggleTests => self.toggle_bottom(BottomTool::Tests),
            ActionId::ToggleSearch => self.toggle_bottom(BottomTool::Search),
            ActionId::ToggleTerminal => self.toggle_bottom(BottomTool::Terminal),
            ActionId::ToggleUsages => self.toggle_bottom(BottomTool::Usages),
            ActionId::NavBack => self.nav_back(),
            ActionId::NavForward => self.nav_forward(),
            ActionId::UsageNext => self.usage_step(1),
            ActionId::UsagePrevious => self.usage_step(-1),
            ActionId::GotoDeclaration => self.nav_at_cursor(crate::nav::Purpose::Jump),
            ActionId::FindUsages => self.nav_at_cursor(crate::nav::Purpose::Usages),
            ActionId::ToggleTheme => {
                let dark = !self.dark;
                self.set_dark(dark);
                // Le thème survit au redémarrage : réglage « Tous mes projets ».
                if self.settings_files.path(Scope::User).is_some() {
                    let value = if dark { "sombre" } else { "clair" };
                    self.write_setting(
                        Scope::User,
                        &["app", "theme"],
                        Some(SettingValue::Text(value.to_owned())),
                    );
                }
            }
            ActionId::Settings => self.open_settings(None),
            ActionId::GitCommit => {
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git.show_changes_for_commit();
            }
            ActionId::ToggleCommit => {
                let visible = self.tools.bottom_open
                    && self.tools.bottom == BottomTool::Git
                    && self.git.tab == GitTab::Changes;
                if visible {
                    self.tools.bottom_open = false;
                    self.focus_editor();
                } else {
                    self.tools.bottom = BottomTool::Git;
                    self.tools.bottom_open = true;
                    self.git.show_changes_for_commit();
                }
            }
            ActionId::BottomTall => self.tools.bottom_tall = !self.tools.bottom_tall,
            ActionId::GitRefresh => {
                self.git.invalidate_diff();
                self.run_git_intents(vec![GitIntent::Refresh]);
            }
            ActionId::TestsRun => self.start_run(RunRequest::all(), false),
            // Ctrl+F5 : relancer la dernière exécution (même demande).
            ActionId::TestsRerun => {
                let request = self
                    .tests
                    .last_request
                    .clone()
                    .unwrap_or_else(RunRequest::all);
                self.start_run(request, false);
            }
            ActionId::TestsStop => self.stop_tests(),
            other => self.run_v1_action(other, None),
        }
        ctx.request_repaint();
    }

    fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        theme::apply(&self.ctx, dark);
        self.editors.set_dark(dark);
        self.git.set_dark(dark);
        if let Some(merge) = &mut self.merge {
            merge.set_dark(dark);
        }
        if let Some(window) = &mut self.diff_window {
            window.pane.set_dark(dark);
        }
    }

    fn toggle_bottom(&mut self, tool: BottomTool) {
        let ctx = self.ctx.clone();
        let visible = self.tools.bottom_open && self.tools.bottom == tool;
        if tool == BottomTool::Terminal {
            let focused = self
                .terminal
                .as_ref()
                .is_some_and(|terminal| terminal.has_focus(&ctx));
            if visible && focused {
                self.tools.bottom_open = false;
                self.focus_editor();
            } else {
                self.tools.bottom = tool;
                self.tools.bottom_open = true;
                if let Some(terminal) = &mut self.terminal {
                    terminal.focus();
                }
            }
        } else if visible {
            self.tools.bottom_open = false;
            self.focus_editor();
        } else {
            self.tools.bottom = tool;
            self.tools.bottom_open = true;
        }
    }

    fn focus_editor(&mut self) {
        if let Some(editor) = self.editors.active_editor_mut() {
            editor.view.request_focus();
        }
    }

    fn selection_prefill(&self) -> Option<String> {
        let editor = self.editors.active_editor()?;
        if editor.document.selections().primary().is_empty() {
            return None;
        }
        editor.document.copy_text().filter(|s| !s.contains('\n'))
    }

    fn open_file(&mut self, path: PathBuf, goto: Option<Goto>) {
        self.user_opens.insert(path.clone());
        self.editors.open(&self.tasks, path, goto);
    }

    fn request_close(&mut self, index: usize) {
        let Some(editor) = self.editors.items.get(index) else {
            return;
        };
        if editor.document.is_modified() {
            if let Some(path) = editor.path() {
                self.dialog = Some(Dialog::CloseTab(path.to_path_buf()));
                return;
            }
        }
        self.editors.close(index);
    }

    fn dispatch_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some()
            || self.merge.is_some()
            || self.diff_window.is_some()
            || self.git_dialog.is_some()
            || self.settings_dialog.is_some()
            || self.fs_dialog.is_some()
        {
            return;
        }
        let terminal_focus = self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.has_focus(ctx));
        // Les raccourcis les plus chargés en modificateurs d'abord.
        let mut candidates: Vec<(ActionId, egui::KeyboardShortcut)> = self
            .registry
            .all()
            .iter()
            .filter(|a| !terminal_focus || a.in_terminal)
            .filter_map(|a| a.shortcut.map(|s| (a.id, s)))
            .collect();
        candidates.sort_by_key(|(_, s)| {
            let m = s.modifiers;
            std::cmp::Reverse(m.ctrl as u8 + m.alt as u8 + m.shift as u8)
        });
        let mut triggered = Vec::new();
        for (id, shortcut) in candidates {
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                triggered.push(id);
            }
        }
        for id in triggered {
            if self.action_enabled(id) {
                self.run_action(id);
            }
        }
        // Boutons précédent / suivant de la souris : l'historique (sauf dans le terminal).
        if !terminal_focus {
            self.nav_mouse_buttons(ctx);
        }
    }

    // ----- Fenêtre : fermeture, titre, relève du disque --------------------------

    fn handle_close_request(&mut self, ctx: &egui::Context) {
        let requested = ctx.input(|i| i.viewport().close_requested());
        if !requested || self.force_close {
            return;
        }
        if self.editors.any_modified() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.dialog.is_none() {
                self.dialog = Some(Dialog::Quit(self.editors.modified_paths()));
            }
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let title = match &self.project {
            Some(project) => match project.branch_name() {
                Some(branch) => {
                    format!("{} [{}] — {}", project.project.name(), branch, t::APP_NAME)
                }
                None => format!("{} — {}", project.project.name(), t::APP_NAME),
            },
            None => t::APP_NAME.to_owned(),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    /// L'état Git se relit toutes les 2,5 s tant que la fenêtre est active (un
    /// changement fait ailleurs, terminal compris, apparaît en moins de 3 s),
    /// sans relancer une lecture déjà en cours.
    /// Relit l'état Git sans attendre la prochaine relève.
    fn refresh_git(&mut self) {
        if let Some(project) = &self.project {
            if let Some(repo) = &project.repo {
                if !self.tracker.kinds().contains(&"vcs.status") {
                    project.vcs.refresh_status(repo);
                    self.last_status_poll = Instant::now();
                }
            }
        }
    }

    fn poll_git(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.focused) || self.scenario_active;
        let Some(project) = &self.project else { return };
        let Some(repo) = &project.repo else { return };
        if !focused {
            return;
        }
        // Un fichier modifié hors de l'IDE doit apparaître en moins de 2 s : la relève
        // dure 1,2 s au plus (elle ne se chevauche jamais), et d'autres événements
        // (changement détecté sur un onglet, retour du focus) la devancent.
        const PERIOD: Duration = Duration::from_millis(1200);
        if self.last_status_poll.elapsed() >= PERIOD {
            self.last_status_poll = Instant::now();
            if !self.tracker.kinds().contains(&"vcs.status") {
                project.vcs.refresh_status(repo);
            }
        }
        let remaining = PERIOD.saturating_sub(self.last_status_poll.elapsed());
        ctx.request_repaint_after(remaining.max(Duration::from_millis(100)));
    }

    fn poll_disk(&mut self, ctx: &egui::Context) {
        /// Relève des fichiers ouverts (un simple contrôle de date et de taille).
        const DISK_PERIOD: Duration = Duration::from_secs(1);
        // En mode scénario la fenêtre n'a pas forcément le focus du système.
        let focused = ctx.input(|i| i.focused) || self.scenario_active;
        let regained = focused && !self.window_focused;
        self.window_focused = focused;
        if self.editors.items.is_empty() || !focused {
            return;
        }
        if regained || self.last_disk_poll.elapsed() >= DISK_PERIOD {
            self.last_disk_poll = Instant::now();
            self.editors.check_disk(&self.tasks);
            if regained {
                self.git.invalidate_diff();
                if let Some(project) = &self.project {
                    if let Some(repo) = &project.repo {
                        project.vcs.refresh_status(repo);
                    }
                }
            }
        }
        let remaining = DISK_PERIOD.saturating_sub(self.last_disk_poll.elapsed());
        ctx.request_repaint_after(remaining.max(Duration::from_millis(50)));
    }

    // ----- Dessin --------------------------------------------------------------

    fn status_model(&self) -> StatusModel {
        let mut model = StatusModel {
            file: None,
            modified: false,
            position: None,
            encoding: None,
            line_ending: None,
            branch: self.project.as_ref().and_then(ProjectState::branch_name),
            ahead_behind: None,
            tasks: self.tracker.visible(),
            message: self.message.as_ref().map(|m| (m.text.clone(), m.error)),
            operation: None,
            progress_text: self
                .net_progress
                .clone()
                .filter(|_| !self.tracker.running_of(status::cancellable).is_empty()),
            message_action: self.message.as_ref().and_then(|m| m.action.as_ref()).map(
                |a| match a {
                    ToastAction::OpenFile(..) => t::SETTINGS_UNREADABLE_OPEN.to_owned(),
                    _ => t::BTN_RESTORE.to_owned(),
                },
            ),
        };
        if let Some(project) = &self.project {
            if let GitInfo::Repository(status) = &project.git {
                model.ahead_behind = status.upstream.as_ref().and_then(|u| u.ahead_behind);
                if let Some(operation) = &status.operation {
                    model.operation = Some(t::operation_conflicts(
                        operation.kind,
                        status.conflicts().count(),
                    ));
                }
            }
        }
        if let Some(editor) = self.editors.active_editor() {
            let document = &editor.document;
            if let Some(path) = document.path() {
                model.file = Some(match &self.project {
                    Some(project) => project
                        .project
                        .relative(path)
                        .map(|r| r.display().to_string())
                        .unwrap_or_else(|| path.display().to_string()),
                    None => path.display().to_string(),
                });
            }
            model.modified = document.is_modified();
            let position = document.cursor_position();
            model.position = Some((position.line + 1, position.column + 1));
            let format = document.format();
            model.encoding = Some(format.encoding.name());
            model.line_ending = Some(match format.line_ending {
                LineEnding::Lf => "LF",
                LineEnding::CrLf => "CRLF",
                LineEnding::Cr => "CR",
                LineEnding::Mixed => t::EOL_MIXED,
            });
        }
        model
    }

    fn draw_menu(&mut self, ui: &mut egui::Ui, p: &Palette) -> Option<ActionId> {
        let mut chosen = None;
        let widget = self.widget_info();
        let widget_tip = self.widget_tooltip();
        let title = self.project.as_ref().map(|project| {
            (
                project.project.name(),
                project.branch_name(),
                project.project.root().display().to_string(),
            )
        });
        egui::Panel::top("menu_bar")
            .frame(
                Frame::new()
                    .fill(p.bg_alt)
                    .inner_margin(Margin::symmetric(8, 2)),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), rect, Icon::Bolt, p.accent);
                    ui.label(egui::RichText::new(t::APP_NAME).strong().color(p.text));
                    ui.add_space(6.0);
                    for (menu, label) in [
                        (Menu::File, t::MENU_FILE),
                        (Menu::Navigate, t::MENU_NAVIGATE),
                        (Menu::View, t::MENU_VIEW),
                        (Menu::Git, t::MENU_GIT),
                        (Menu::Tests, t::MENU_TESTS),
                    ] {
                        ui.menu_button(label, |ui| {
                            ui.set_min_width(260.0);
                            for action in
                                self.registry.all().iter().filter(|a| a.menu == Some(menu))
                            {
                                let mut button = egui::Button::new(action.label);
                                if let Some(text) = self.registry.shortcut_text(action.id) {
                                    button = button.shortcut_text(text);
                                }
                                let enabled = self.action_enabled(action.id);
                                if ui.add_enabled(enabled, button).clicked() {
                                    chosen = Some(action.id);
                                    ui.close();
                                }
                            }
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some((name, _, root)) = &title {
                            ui.label(egui::RichText::new(name).color(p.text_dim))
                                .on_hover_text(root);
                        }
                        // Le widget de branche : nom, avance et retard ; un clic ouvre la fenêtre.
                        if let Some(info) = &widget {
                            let response =
                                branchui::widget(ui, p, info, self.branch_popup.is_some());
                            self.branch_widget = Some(response.rect);
                            if response.clicked() {
                                chosen = Some(ActionId::BranchesPopup);
                            }
                            response.on_hover_text(widget_tip.as_str());
                        } else {
                            self.branch_widget = None;
                        }
                    });
                });
            });
        chosen
    }

    fn draw_bottom(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if !self.tools.bottom_open {
            return;
        }
        let tools = self.tools;
        let mut selected_tab = None;
        let mut collapse = false;
        let mut open_file: Option<(PathBuf, Option<Goto>)> = None;
        let mut git_intents = Vec::new();
        let mut tests_intents = Vec::new();
        let mut usages_intents = Vec::new();
        let indexing = self
            .project
            .as_ref()
            .and_then(|p| p.index.symbols_progress());
        let has_usages = !self.usages.is_empty();
        let repo_state = self.repo_state();
        let shortcut = |id: ActionId| self.registry.shortcut_text(id).unwrap_or_default();
        let commit_shortcut = shortcut(ActionId::GitCommit);
        let run_shortcut = shortcut(ActionId::TestsRun);
        let stop_shortcut = shortcut(ActionId::TestsStop);
        let screen_height = ui.ctx().content_rect().height();
        let (panel_id, default_height) = if tools.bottom_tall {
            ("bottom_tools_tall", (screen_height * 0.62).max(300.0))
        } else {
            ("bottom_tools", 280.0)
        };
        egui::Panel::bottom(panel_id)
            .resizable(true)
            .default_size(default_height)
            .min_size(110.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(0, 0)),
            )
            .show(ui, |ui| {
                let (clicked, close) = tools::bottom_header(ui, p, &tools, has_usages);
                selected_tab = clicked;
                collapse = close;
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.cursor().top(),
                    egui::Stroke::new(1.0, p.border),
                );
                match tools.bottom {
                    BottomTool::Terminal => match &mut self.terminal {
                        Some(terminal) => {
                            for intent in terminal.show(ui) {
                                let TerminalIntent::OpenFile { path, line, column } = intent;
                                let path = crate::terminal::resolve_path(terminal.cwd(), &path);
                                let goto = line.map(|line| Goto {
                                    position: TextPosition::new(
                                        line.saturating_sub(1),
                                        column.unwrap_or(1).saturating_sub(1),
                                    ),
                                    end: None,
                                });
                                open_file = Some((path, goto));
                            }
                        }
                        None => {
                            ui.add_space(8.0);
                            ui.weak(t::TERMINAL_NO_PROJECT);
                        }
                    },
                    BottomTool::Search => {
                        if let Some(find_in_files::Outcome::Open { path, line, range }) =
                            self.find.show_results_tab(ui, p)
                        {
                            open_file = Some((path, goto_for(line, range)));
                        }
                    }
                    BottomTool::Git => {
                        git_intents = self.git.show(
                            ui,
                            p,
                            repo_state,
                            &commit_shortcut,
                            self.branches.as_ref(),
                        );
                    }
                    BottomTool::Tests => {
                        tests_intents = self.tests.show(ui, p, &run_shortcut, &stop_shortcut);
                    }
                    BottomTool::Usages => {
                        usages_intents = self.usages.show(ui, p, indexing);
                    }
                }
            });
        if let Some(tool) = selected_tab {
            self.tools.bottom = tool;
            if tool == BottomTool::Terminal {
                if let Some(terminal) = &mut self.terminal {
                    terminal.focus();
                }
            }
        }
        if collapse {
            self.tools.bottom_open = false;
            self.focus_editor();
        }
        if let Some((path, goto)) = open_file {
            self.open_file(path, goto);
        }
        let intents = std::mem::take(&mut git_intents);
        self.run_git_intents(intents);
        let intents = std::mem::take(&mut tests_intents);
        self.run_tests_intents(intents);
        self.usages_intents(usages_intents);
    }

    fn draw_left(&mut self, ui: &mut egui::Ui, p: &Palette) -> Vec<ActionId> {
        let tools = self.tools;
        let mut actions = Vec::new();
        egui::Panel::left("tool_strip")
            .exact_size(42.0)
            .resizable(false)
            .frame(Frame::new().fill(p.bg_alt))
            .show_separator_line(false)
            .show(ui, |ui| {
                actions = tools::side_strip(ui, p, &tools, &self.registry);
            });
        actions
    }

    fn draw_project_panel(&mut self, ui: &mut egui::Ui, p: &Palette) {
        if !self.tools.project_open {
            return;
        }
        let name = self.project.as_ref().map(|project| project.project.name());
        let mut to_open = None;
        egui::Panel::left("project_panel")
            .resizable(true)
            .default_size(270.0)
            .min_size(150.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(0, 0)),
            )
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(t::TOOL_PROJECT.to_uppercase())
                            .small()
                            .strong()
                            .color(p.text_dim),
                    );
                });
                ui.add_space(4.0);
                if name.is_some() {
                    to_open = self.tree.show(ui, p);
                } else {
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        ui.weak(t::TREE_NO_PROJECT);
                    });
                }
            });
        if let Some(path) = to_open {
            self.open_file(path, None);
        }
    }

    fn draw_central(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let editor_bg = theme::editor_theme(self.dark).background;
        let mut tab_intents = Vec::new();
        let mut banner = None;
        let mut editor_intents = Vec::new();
        let mut welcome_action = None;
        // Le mot navigable souligné (survol avec Ctrl) ne concerne que le fichier affiché.
        let link = self
            .nav
            .hover
            .link
            .as_ref()
            .filter(|l| self.editors.active_editor().and_then(|e| e.path()) == Some(&l.path))
            .map(|l| l.range);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(editor_bg))
            .show(ui, |ui| {
                if self.editors.items.is_empty() {
                    welcome_action = self.draw_welcome(ui, p);
                } else {
                    tab_intents = self.editors.show_tab_bar(ui, p, editor_bg);
                    (banner, editor_intents) = self.editors.show_active(ui, p, link);
                }
            });
        for intent in tab_intents {
            match intent {
                TabIntent::Activate(i) => self.editors.activate(i),
                TabIntent::Close(i) => self.request_close(i),
            }
        }
        if let Some(BannerIntent::Reload(path)) = banner {
            julie_text::spawn_load(&self.tasks, path);
        }
        self.nav_editor_intents(editor_intents);
        if let Some(action) = welcome_action {
            self.run_action(action);
        }
    }

    fn draw_welcome(&self, ui: &mut egui::Ui, p: &Palette) -> Option<ActionId> {
        let mut chosen = None;
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.28).max(20.0));
            let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            icons::paint(ui.painter(), rect, Icon::Bolt, p.accent);
            ui.label(
                egui::RichText::new(t::APP_NAME)
                    .size(26.0)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(14.0);
            if self.project.is_none() {
                ui.label(egui::RichText::new(t::WELCOME_NO_PROJECT).color(p.text_dim));
                ui.add_space(8.0);
                let id = ActionId::OpenFolder;
                if ui.button(self.registry.get(id).label).clicked() {
                    chosen = Some(id);
                }
            }
            ui.add_space(10.0);
            for id in [
                ActionId::GotoFile,
                ActionId::FindInFiles,
                ActionId::ActionPalette,
                ActionId::OpenFolder,
            ] {
                let action = self.registry.get(id);
                let shortcut = self.registry.shortcut_text(id).unwrap_or_default();
                ui.label(
                    egui::RichText::new(format!("{}    {}", action.label, shortcut))
                        .color(p.text_dim),
                );
            }
        });
        chosen
    }

    fn draw_overlays(&mut self, ctx: &egui::Context, p: &Palette) {
        // Aller au fichier.
        if let Some(goto) = &mut self.goto {
            let outcome = goto.show(
                ctx,
                p,
                self.project.as_ref().map(|project| &*project.index),
                &self.editors.recent,
            );
            match outcome {
                goto_file::Outcome::Pending => {}
                goto_file::Outcome::Close => {
                    self.goto = None;
                    self.focus_editor();
                }
                goto_file::Outcome::Open { path, position } => {
                    self.goto = None;
                    let goto = position.map(|position| Goto {
                        position,
                        end: None,
                    });
                    self.open_file(path, goto);
                }
            }
        }
        // Recherche dans les fichiers.
        if self.find_open {
            let outcome = self.find.show(
                ctx,
                p,
                self.project.as_ref().map(|project| &*project.index),
                &self.editors,
            );
            match outcome {
                find_in_files::Outcome::Pending => {}
                find_in_files::Outcome::Close => {
                    self.find_open = false;
                    if let Some(project) = &self.project {
                        project.index.cancel_search();
                    }
                    self.focus_editor();
                }
                find_in_files::Outcome::Open { path, line, range } => {
                    self.find_open = false;
                    self.open_file(path, goto_for(line, range));
                }
            }
        }
        // Palette d'actions.
        if let Some(mut palette) = self.palette.take() {
            let outcome = {
                let enabled = |id: ActionId| self.action_enabled(id);
                palette.show(ctx, p, &self.registry, &enabled)
            };
            match outcome {
                palette::Outcome::Pending => self.palette = Some(palette),
                palette::Outcome::Close => {
                    self.palette = None;
                    self.focus_editor();
                }
                palette::Outcome::Run(id) => {
                    self.palette = None;
                    self.run_action(id);
                }
            }
        }
    }

    /// Dialogue de fusion et grande fenêtre de diff.
    fn draw_modals(&mut self, ctx: &egui::Context, p: &Palette) {
        if let Some(dialog) = &mut self.merge {
            let intents = dialog.show(ctx, p);
            self.run_merge_intents(intents);
        }
        let mut close = false;
        if let Some(window) = &mut self.diff_window {
            if self.merge.is_none() {
                close = diff_window_ui(ctx, p, window);
            }
        }
        if close {
            self.diff_window = None;
        }
    }

    fn draw_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(dialog) = &self.dialog else { return };
        enum Answer {
            Save,
            Discard,
            Cancel,
        }
        let (title, detail) = match dialog {
            Dialog::CloseMany(paths) => (
                t::close_many_title(paths.len()),
                paths.iter().map(|p| file_name(p)).collect::<Vec<_>>().join(
                    "
",
                ),
            ),
            Dialog::CloseTab(path) => (t::dialog_save_one(&file_name(path)), String::new()),
            Dialog::Quit(paths) => (
                t::DIALOG_SAVE_MANY.to_owned(),
                paths
                    .iter()
                    .map(|p| file_name(p))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        };
        let quitting = matches!(dialog, Dialog::Quit(_) | Dialog::CloseMany(_));
        let mut answer = None;
        let modal = egui::Modal::new(Id::unique("julie-dialog")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new(title).strong().size(15.0).color(p.text));
            if !detail.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(detail).color(p.text_dim));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let save = ui.button(if quitting {
                    t::DIALOG_SAVE_ALL
                } else {
                    t::DIALOG_SAVE
                });
                if ui.memory(|m| m.focused().is_none()) {
                    save.request_focus();
                }
                if save.clicked() {
                    answer = Some(Answer::Save);
                }
                if ui.button(t::DIALOG_DISCARD).clicked() {
                    answer = Some(Answer::Discard);
                }
                if ui.button(t::DIALOG_CANCEL).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(Answer::Cancel);
        }
        let Some(answer) = answer else { return };
        let dialog = self.dialog.take();
        self.focus_editor();
        match (answer, dialog) {
            (Answer::Cancel, _) => {}
            (Answer::Save, Some(Dialog::CloseTab(path))) => {
                let errors = self
                    .editors
                    .save_paths(&self.tasks, std::slice::from_ref(&path));
                if errors.is_empty() {
                    self.editors.close_after_save.insert(path);
                } else {
                    for error in errors {
                        self.say(error, true);
                    }
                }
            }
            (Answer::Discard, Some(Dialog::CloseTab(path))) => {
                if let Some(index) = self.editors.index_of(&path) {
                    self.editors.close(index);
                }
            }
            (Answer::Save, Some(Dialog::Quit(paths))) => {
                let errors = self.editors.save_paths(&self.tasks, &paths);
                if errors.is_empty() {
                    self.quit_after_save = true;
                } else {
                    for error in errors {
                        self.say(error, true);
                    }
                }
            }
            (Answer::Discard, Some(Dialog::Quit(_))) => {
                self.force_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Answer::Save, Some(Dialog::CloseMany(paths))) => {
                let errors = self.editors.save_paths(&self.tasks, &paths);
                if errors.is_empty() {
                    self.editors.close_after_save.extend(paths);
                } else {
                    for error in errors {
                        self.say(error, true);
                    }
                }
            }
            (Answer::Discard, Some(Dialog::CloseMany(paths))) => {
                for path in paths {
                    if let Some(index) = self.editors.index_of(&path) {
                        self.editors.close(index);
                    }
                }
            }
            (_, None) => {}
        }
    }

    // ----- Mode scénario -------------------------------------------------------

    fn load_scenario(&mut self, file: &Path) {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed = std::fs::read_to_string(file)
            .map_err(|e| format!("{} : {e}", file.display()))
            .and_then(|text| scenario::parse(&text));
        match parsed {
            Ok(steps) => {
                self.scenario = Some(Scenario::new(name, steps));
                self.scenario_active = true;
            }
            Err(error) => {
                eprintln!("[scénario] {error}");
                scenario::FAILED.store(true, Ordering::SeqCst);
                self.force_close = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Plus rien en cours : le scénario peut passer à l'étape suivante.
    fn is_idle(&self) -> bool {
        self.opening == 0
            && !self.tracker.busy()
            && self
                .project
                .as_ref()
                .is_none_or(|p| p.index_ready && !matches!(p.git, GitInfo::Unknown))
            && !self.find.running
            && self.find.is_settled()
            && self.goto.as_ref().is_none_or(GotoFile::is_settled)
            && self.editors.is_loaded()
            && self
                .project
                .as_ref()
                .is_none_or(|p| p.index.symbols_progress().is_none())
            && !self.nav.busy()
            && !self.usages.running()
            && !self.unsaved_pending()
            && !self.tree.is_loading()
            && self.git.merge_loading.is_none()
            && self
                .settings_dialog
                .as_ref()
                .is_none_or(SettingsDialog::settled)
    }

    fn scenario_step(&mut self, ctx: &egui::Context) {
        let Some(mut sc) = self.scenario.take() else {
            return;
        };
        ctx.request_repaint_after(Duration::from_millis(16));
        loop {
            if sc.finished {
                break;
            }
            if sc.settle > 0 {
                sc.settle -= 1;
                break;
            }
            if let Some(until) = sc.wait_until {
                if Instant::now() < until {
                    break;
                }
                sc.wait_until = None;
            }
            if let Some(flag) = &sc.screenshot {
                if !flag.load(Ordering::SeqCst) {
                    break;
                }
                sc.screenshot = None;
            }
            if sc.waiting_idle {
                if sc.idle_since.elapsed() > Duration::from_secs(60) {
                    sc.fail(0, "wait-idle : délai de 60 s dépassé");
                    sc.waiting_idle = false;
                } else if self.is_idle() {
                    sc.idle_frames += 1;
                    if sc.idle_frames < 4 {
                        break;
                    }
                    sc.waiting_idle = false;
                } else {
                    sc.idle_frames = 0;
                    break;
                }
            }
            let Some((line, step)) = sc.next() else {
                // Fin du scénario : on quitte sans rien demander.
                self.quit_for_scenario(ctx);
                sc.finished = true;
                break;
            };
            self.exec_step(line, step, &mut sc, ctx);
        }
        self.scenario = Some(sc);
    }

    fn quit_for_scenario(&mut self, ctx: &egui::Context) {
        self.force_close = true;
        if let Some(terminal) = &mut self.terminal {
            terminal.close_all();
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn exec_step(&mut self, line: usize, step: Step, sc: &mut Scenario, ctx: &egui::Context) {
        sc.settle = 3;
        match step {
            Step::Project { path, wait } => {
                self.start_open(path);
                if wait {
                    sc.start_idle_wait();
                }
            }
            Step::Fullscreen(on) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
                sc.settle = 10;
            }
            Step::File {
                path,
                line: l,
                column,
                wait,
            } => {
                let path = match &self.project {
                    Some(project) if path.is_relative() => project.project.root().join(path),
                    _ => path,
                };
                let goto = l.map(|l| Goto {
                    position: TextPosition::new(
                        l.saturating_sub(1),
                        column.unwrap_or(1).saturating_sub(1),
                    ),
                    end: None,
                });
                self.open_file(path, goto);
                if wait {
                    sc.start_idle_wait();
                } else {
                    // Le temps que le fichier arrive (un chargement n'est pas de l'indexation).
                    sc.wait_ms(300);
                }
            }
            Step::Action(key) => match self.registry.by_key(&key) {
                Some(id) => {
                    if self.action_enabled(id) {
                        self.run_action(id);
                    } else {
                        sc.fail(
                            line,
                            &format!("action « {key} » indisponible dans cet état"),
                        );
                    }
                }
                None => sc.fail(line, &format!("action inconnue : « {key} »")),
            },
            Step::Type(text) => self.injected.push_back(Injection {
                events: vec![egui::Event::Text(text)],
            }),
            Step::Key(combo) => self.injected.push_back(Injection {
                events: combo.events(),
            }),
            Step::Click(x, y, mods) => {
                self.injected.extend(Injection::click_with(
                    egui::pos2(x, y),
                    egui::PointerButton::Primary,
                    mods,
                ));
                sc.settle = 5;
            }
            Step::RClick(x, y, mods) => {
                self.injected.extend(Injection::click_with(
                    egui::pos2(x, y),
                    egui::PointerButton::Secondary,
                    mods,
                ));
                sc.settle = 5;
            }
            Step::Wait(ms) => sc.wait_ms(ms),
            Step::WaitIdle => sc.start_idle_wait(),
            Step::Screenshot(path) => {
                let done = Arc::new(AtomicBool::new(false));
                sc.screenshot = Some(done.clone());
                let name = sc.name.clone();
                ctx.request_screenshot(move |image| {
                    if let Err(error) = write_png(&path, &image) {
                        eprintln!("[scénario {name}] capture impossible : {error}");
                        scenario::FAILED.store(true, Ordering::SeqCst);
                    }
                    done.store(true, Ordering::SeqCst);
                });
            }
            Step::State(path) => {
                let json = self.state_json(ctx);
                let result = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| {
                        std::fs::write(
                            &path,
                            serde_json::to_string_pretty(&json).unwrap_or_default(),
                        )
                    });
                if let Err(error) = result {
                    sc.fail(
                        line,
                        &format!("état non écrit ({}) : {error}", path.display()),
                    );
                }
            }
            Step::Resize(w, h) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(w, h)));
                sc.settle = 8;
            }
            Step::Scale(factor) => {
                ctx.set_pixels_per_point(factor);
                sc.settle = 6;
            }
            Step::Theme(dark) => self.set_dark(dark),
            Step::Ui { command, arg } => {
                if let Err(error) = self.ui_command(&command, &arg) {
                    sc.fail(line, &format!("ui {command} : {error}"));
                }
                sc.settle = 8;
            }
            Step::TreeExpand(relative) => {
                if let Some(project) = &self.project {
                    let path = project.project.root().join(relative.replace('/', "\\"));
                    self.tree.expand_path(&path);
                    self.tools.project_open = true;
                } else {
                    sc.fail(line, "tree-expand sans projet ouvert");
                }
                sc.start_idle_wait();
            }
            Step::Log(text) => eprintln!("[scénario {}:{line}] {text}", sc.name),
            Step::ExternalWrite { path, text } => {
                let path = match &self.project {
                    Some(project) if path.is_relative() => project.project.root().join(path),
                    _ => path,
                };
                let temp = std::env::temp_dir();
                if !path.starts_with(&temp) {
                    sc.fail(
                        line,
                        &format!("external-write refusé hors de {}", temp.display()),
                    );
                } else if let Err(error) = std::fs::write(&path, text) {
                    sc.fail(line, &format!("écriture impossible : {error}"));
                }
            }
            Step::Quit => {
                self.quit_for_scenario(ctx);
                sc.finished = true;
            }
        }
    }

    fn state_json(&self, ctx: &egui::Context) -> serde_json::Value {
        let focus = if self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.has_focus(ctx))
        {
            "terminal"
        } else if self.tree.has_focus(ctx) {
            "tree"
        } else if self
            .editors
            .active_editor()
            .is_some_and(|e| e.view.has_focus(ctx))
        {
            "editor"
        } else {
            "none"
        };
        let titles = self.editors.titles();
        let tabs: Vec<serde_json::Value> = self
            .editors
            .items
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let position = e.document.cursor_position();
                serde_json::json!({
                    "title": titles[i],
                    "path": e.path().map(|p| p.display().to_string()),
                    "active": i == self.editors.active,
                    "modified": e.document.is_modified(),
                    "line": position.line + 1,
                    "column": position.column + 1,
                    "notice": e.notice.map(|n| format!("{n:?}")),
                    "read_only": e.document.is_read_only(),
                    "lines": e.document.buffer().len_lines(),
                    "first_line": e.document.buffer().line(0).to_string(),
                    "language": format!("{:?}", e.document.language()),
                })
            })
            .collect();
        let mut value = serde_json::json!({
            "title": self.last_title,
            "dark": self.dark,
            "pixels_per_point": ctx.pixels_per_point(),
            "window": { "width": ctx.content_rect().width(), "height": ctx.content_rect().height() },
            "focus": focus,
            "project": self.project.as_ref().map(|p| serde_json::json!({
                "root": p.project.root().display().to_string(),
                "name": p.project.name(),
                "index_ready": p.index_ready,
                "git": match &p.git {
                    GitInfo::Unknown => serde_json::json!("unknown"),
                    GitInfo::NoRepository => serde_json::json!("no_repository"),
                    GitInfo::Repository(s) => serde_json::json!({
                        "branch": p.branch_name(),
                        "changed_files": s.files.len(),
                        "upstream": s.upstream.as_ref().map(|u| u.name.clone()),
                    }),
                },
            })),
            "tabs": tabs,
            "recent": self.editors.recent.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "tree": {
                "visible": self.tree.visible_names(),
                "expanded": self.tree.expanded_dirs().iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "selected": self.tree.selected.as_ref().map(|p| p.display().to_string()),
            },
            "tools": {
                "project": self.tools.project_open,
                "bottom_open": self.tools.bottom_open,
                "bottom": self.tools.bottom.key(),
                "bottom_tall": self.tools.bottom_tall,
            },
            "git": self.git.state(),
            "merge": self.merge.as_ref().map(MergeDialog::state),
            "diff_window": self.diff_window.as_ref().map(|w| w.pane.state()),
            "tests": self.tests.state(),
            "nav": self.nav_state(),
            "usages": self.usages_state(),
            "goto": self.goto.as_ref().map(GotoFile::state),
            "find": { "open": self.find_open, "details": self.find.state() },
            "palette": self.palette.as_ref().map(|p| serde_json::json!({
                "query": p.query, "selected": p.selected,
                "matches": p.matches(&self.registry).iter().map(|id| self.registry.get(*id).key).collect::<Vec<_>>(),
            })),
            "fs_dialog": self.fs_dialog.as_ref().map(FsDialog::state),
            "dialog": self.dialog.as_ref().map(|d| match d {
                Dialog::CloseTab(p) => format!("close_tab:{}", p.display()),
                Dialog::Quit(p) => format!("quit:{}", p.len()),
                Dialog::CloseMany(p) => format!("close_many:{}", p.len()),
            }),
            "status": self.status_model().to_json(),
            "tasks_busy": self.tracker.kinds(),
        });
        if let (serde_json::Value::Object(map), serde_json::Value::Object(extra)) =
            (&mut value, self.v1_state())
        {
            map.extend(extra);
        }
        value
    }
}

/// La grande fenêtre de diff : rend `true` quand on la ferme (Échap, croix).
fn diff_window_ui(ctx: &egui::Context, p: &Palette, window: &mut DiffWindow) -> bool {
    let screen = ctx.content_rect();
    let margin = if screen.width() > 1500.0 { 40.0 } else { 18.0 };
    let size = vec2(
        (screen.width() - 2.0 * margin).max(600.0),
        (screen.height() - 2.0 * margin).max(360.0),
    );
    let frame = Frame::new()
        .fill(p.bg)
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(10)
        .shadow(egui::Shadow {
            offset: [0, 10],
            blur: 36,
            spread: 2,
            color: egui::Color32::from_black_alpha(130),
        });
    let mut close = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    crate::kit::modal("diff-fenetre")
        .frame(frame)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let bar = egui::Rect::from_min_size(rect.min, vec2(rect.width(), 36.0));
            ui.painter().rect_filled(
                bar,
                egui::CornerRadius {
                    nw: 10,
                    ne: 10,
                    sw: 0,
                    se: 0,
                },
                p.bg_alt,
            );
            ui.painter().text(
                egui::pos2(rect.left() + 16.0, bar.center().y),
                egui::Align2::LEFT_CENTER,
                &window.pane.title,
                egui::FontId::proportional(14.5),
                p.text,
            );
            ui.painter().text(
                egui::pos2(rect.right() - 48.0, bar.center().y),
                egui::Align2::RIGHT_CENTER,
                t::DIFF_CLOSE_HINT,
                egui::FontId::proportional(12.0),
                p.text_dim,
            );
            let button = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 22.0, bar.center().y),
                vec2(26.0, 26.0),
            );
            if icons::tool_button(
                ui,
                button,
                Id::unique("diff-fermer"),
                Icon::Close,
                p,
                t::CLOSE,
                true,
            ) {
                close = true;
            }
            let body = egui::Rect::from_min_max(egui::pos2(rect.left(), bar.bottom()), rect.max);
            window.pane.show(ui, body, p, true);
        });
    close
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn goto_for(line: u32, range: Option<julie_core::TextRange>) -> Option<Goto> {
    Some(match range {
        Some(range) => Goto {
            position: range.start,
            end: Some(range.end),
        },
        None => Goto {
            position: TextPosition::new(line, 0),
            end: None,
        },
    })
}

fn write_png(path: &Path, image: &egui::ColorImage) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.size[0] as u32,
        image.size[1] as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    writer.write_image_data(&bytes).map_err(|e| e.to_string())
}

impl eframe::App for JulieStormApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(injection) = self.injected.pop_front() {
            raw_input.events.extend(injection.events);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump_messages();
        self.pump_bus();
        self.pump_settings();
        self.scenario_step(&ctx);
        self.handle_close_request(&ctx);
        self.dispatch_shortcuts(&ctx);
        self.poll_disk(&ctx);
        self.poll_git(&ctx);
        self.update_title(&ctx);
        if self
            .message
            .as_ref()
            .is_some_and(|m| m.at.elapsed() > m.lifetime())
        {
            self.message = None;
        } else if self.message.is_some() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        if let Some(delay) = self.tracker.next_reveal() {
            ctx.request_repaint_after(delay);
        }

        let p = self.palette();
        let mut actions = Vec::new();
        let mut status_out = status::StatusOut::default();
        actions.extend(self.draw_menu(ui, &p));

        egui::Panel::bottom("status_bar")
            .exact_size(24.0)
            .frame(
                Frame::new()
                    .fill(p.bg_alt)
                    .inner_margin(Margin::symmetric(10, 0)),
            )
            .show(ui, |ui| {
                let model = self.status_model();
                status_out = status::show(ui, &p, &model);
            });
        if let Some(task) = status_out.cancel {
            self.tasks.cancel(task);
        }
        if status_out.message_action {
            self.toast_action();
        }

        actions.extend(self.draw_left(ui, &p));
        self.nav_popup_keys(&ctx);
        self.draw_bottom(ui, &p);
        self.draw_project_panel(ui, &p);
        self.draw_central(ui, &p);
        self.nav_tick(&ctx);

        // Échap dans l'arbre : retour à l'éditeur.
        if self.tree.has_focus(&ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.focus_editor();
        }

        self.collect_menu_requests();
        self.draw_overlays(&ctx, &p);
        self.draw_nav_popup(&ctx, &p);
        self.draw_hover_tooltip(&ctx, &p);
        self.draw_branch_popup(&ctx, &p);
        self.draw_ctx_menu(&ctx, &p);
        if let Some(terminal) = &mut self.terminal {
            terminal.ui_global(ui);
        }
        self.draw_modals(&ctx, &p);
        self.draw_dialog(&ctx, &p);
        self.draw_git_dialog(&ctx, &p);
        self.draw_settings(&ctx, &p);
        self.draw_fs_dialog(&ctx, &p);

        for id in actions {
            self.run_action(id);
        }
    }

    fn on_exit(&mut self) {
        self.tasks.cancel_all();
    }
}
