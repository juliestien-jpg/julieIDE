//! L'application : état global, boucle d'affichage, distribution des
//! événements du bus et des raccourcis.
//!
//! Règles : aucune règle métier ici (l'interface affiche ce que les crates
//! publient et leur renvoie des intentions) ; rien de bloquant dans `ui` ;
//! redessin uniquement sur événement (sauf pendant un scénario, qui demande ses
//! images, et la relève du disque toutes les 2 s tant qu'un fichier est ouvert).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, vec2, Frame, Id, Key, Margin, Modifiers, Sense};
use julie_core::bus::Bus;
use julie_core::file::LineEnding;
use julie_core::{Domain, DomainEvent, Event, Settings, TaskId, Tasks, TextPosition};
use julie_index::{IndexEvent, IndexService, IndexSettings};
use julie_terminal::{TerminalEvent, TerminalSettings};
use julie_text::{TextError, TextEvent};
use julie_vcs::{Vcs, VcsEvent, VcsSettings};

use crate::actions::{ActionId, Menu, Registry};
use crate::icons::{self, Icon};
use crate::project::{self, Channel, GitInfo, Msg, ProjectState};
use crate::scenario::{self, Injection, Scenario, Step};
use crate::search::find_in_files::{self, FindInFiles};
use crate::search::goto_file::{self, GotoFile};
use crate::search::palette::{self, ActionPalette};
use crate::status::{self, StatusModel, TaskTracker};
use crate::tabs::{BannerIntent, Editors, Goto, TabIntent};
use crate::terminal::{TerminalIntent, TerminalPanel};
use crate::texts as t;
use crate::theme::{self, Palette};
use crate::tools::{self, BottomTool, ToolWindows};
use crate::tree::ProjectTree;

/// Ce que la ligne de commande demande.
pub struct Options {
    pub scenario: Option<PathBuf>,
    pub paths: Vec<PathBuf>,
}

enum Dialog {
    /// Fermer un onglet modifié.
    CloseTab(PathBuf),
    /// Quitter avec des onglets modifiés.
    Quit(Vec<PathBuf>),
}

struct Message {
    text: String,
    error: bool,
    at: Instant,
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
    last_disk_poll: Instant,
    force_close: bool,
    quit_after_save: bool,
    scenario: Option<Scenario>,
    scenario_active: bool,
    injected: VecDeque<Injection>,
    remember_last: bool,
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
            last_disk_poll: Instant::now(),
            force_close: false,
            quit_after_save: false,
            scenario: None,
            scenario_active: false,
            injected: VecDeque::new(),
            remember_last: options.scenario.is_none() || project::state_dir_overridden(),
        };

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
        self.message = Some(Message {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    // ----- Projet --------------------------------------------------------------

    fn start_open(&mut self, path: PathBuf) {
        let path = std::path::absolute(&path).unwrap_or(path);
        self.opening += 1;
        project::spawn_open(path, self.channel.clone());
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
            }
        }
    }

    fn on_project_ready(
        &mut self,
        project: julie_core::Project,
        settings: Result<Settings, String>,
    ) {
        let settings = match settings {
            Ok(settings) => settings,
            Err(error) => {
                self.say(t::settings_failed(&error), true);
                Settings::default()
            }
        };
        let index_settings: IndexSettings = settings.section("index").unwrap_or_default();
        let vcs_settings: VcsSettings = settings.section("vcs").unwrap_or_default();
        let terminal_settings: TerminalSettings = settings.section("terminal").unwrap_or_default();

        if let Some(old) = &self.project {
            old.index.cancel_search();
        }
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
                Event::ProjectOpened(_) | Event::FilesChangedOnDisk(_) => {}
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
                    self.on_vcs(event);
                }
            }
            Domain::Diff | Domain::PhpUnit => {}
        }
    }

    fn on_text(&mut self, event: TextEvent) {
        match event {
            TextEvent::Loaded(loaded) => {
                self.editors.on_loaded(loaded, &self.waker, self.dark);
            }
            TextEvent::LoadFailed { path, error } => {
                self.editors.on_load_failed(&path);
                let reason = match &error {
                    TextError::Binary { .. } => t::ERR_BINARY.to_owned(),
                    TextError::TooLarge { .. } => t::ERR_TOO_LARGE.to_owned(),
                    other => other.to_string(),
                };
                self.say(t::load_failed(&path, &reason), true);
            }
            TextEvent::Saved(outcome) => {
                self.editors.on_saved(&outcome);
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
            TextEvent::DiskChecked(states) => self.editors.on_disk_checked(states, &self.tasks),
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
        }
    }

    fn on_vcs(&mut self, event: &VcsEvent) {
        let Some(project) = &mut self.project else {
            return;
        };
        match event {
            VcsEvent::Opened(repo) => project.repo = Some(repo.clone()),
            VcsEvent::NoRepository { .. } => {
                project.git = GitInfo::NoRepository;
                self.tree.set_git(None);
            }
            VcsEvent::Status(status) => {
                self.tree.set_git(Some(status));
                project.git = GitInfo::Repository(Box::new(status.clone()));
            }
            VcsEvent::Failed(failure) => {
                let text = t::git_failed(&failure.error.to_string());
                self.say(text, true);
            }
            // Journal, diff, conflits, console : vues de la passe 2.
            _ => {}
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
            _ => true,
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
            ActionId::ToggleTheme => {
                self.dark = !self.dark;
                theme::apply(&ctx, self.dark);
                self.editors.set_dark(self.dark);
            }
        }
        ctx.request_repaint();
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
        if self.dialog.is_some() {
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

    fn poll_disk(&mut self, ctx: &egui::Context) {
        // En mode scénario la fenêtre n'a pas forcément le focus du système.
        let focused = ctx.input(|i| i.focused) || self.scenario_active;
        let regained = focused && !self.window_focused;
        self.window_focused = focused;
        if self.editors.items.is_empty() || !focused {
            return;
        }
        if regained || self.last_disk_poll.elapsed() >= Duration::from_secs(2) {
            self.last_disk_poll = Instant::now();
            self.editors.check_disk(&self.tasks);
            if regained {
                if let Some(project) = &self.project {
                    if let Some(repo) = &project.repo {
                        project.vcs.refresh_status(repo);
                    }
                }
            }
        }
        let remaining = Duration::from_secs(2).saturating_sub(self.last_disk_poll.elapsed());
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
        };
        if let Some(project) = &self.project {
            if let GitInfo::Repository(status) = &project.git {
                model.ahead_behind = status.upstream.as_ref().and_then(|u| u.ahead_behind);
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
                        if let Some((name, branch, root)) = &title {
                            let text = match branch {
                                Some(branch) => format!("{name}  ·  {branch}"),
                                None => name.clone(),
                            };
                            ui.label(egui::RichText::new(text).color(p.text_dim))
                                .on_hover_text(root);
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
        egui::Panel::bottom("bottom_tools")
            .resizable(true)
            .default_size(260.0)
            .min_size(110.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(0, 0)),
            )
            .show(ui, |ui| {
                let (clicked, close) = tools::bottom_header(ui, p, &tools);
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
                    other => {
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| {
                            ui.weak(other.empty_text());
                        });
                        // Même hauteur que le terminal à l'ouverture.
                        ui.allocate_space(ui.available_size());
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
        let mut welcome_action = None;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(editor_bg))
            .show(ui, |ui| {
                if self.editors.items.is_empty() {
                    welcome_action = self.draw_welcome(ui, p);
                } else {
                    tab_intents = self.editors.show_tab_bar(ui, p, editor_bg);
                    banner = self.editors.show_active(ui, p);
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
        if let Some(palette) = &mut self.palette {
            let outcome = {
                let enabled = |id: ActionId| match id {
                    ActionId::SaveAll => self.editors.any_modified(),
                    ActionId::CloseTab | ActionId::LastTab => !self.editors.items.is_empty(),
                    ActionId::NextTab | ActionId::PreviousTab => self.editors.items.len() > 1,
                    ActionId::GotoFile | ActionId::FindInFiles => self.project.is_some(),
                    _ => true,
                };
                palette.show(ctx, p, &self.registry, &enabled)
            };
            match outcome {
                palette::Outcome::Pending => {}
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

    fn draw_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(dialog) = &self.dialog else { return };
        enum Answer {
            Save,
            Discard,
            Cancel,
        }
        let (title, detail) = match dialog {
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
        let quitting = matches!(dialog, Dialog::Quit(_));
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
            && !self.tree.is_loading()
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
            terminal.shutdown(Duration::from_millis(1500));
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
                sc.start_idle_wait();
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
            Step::Click(x, y) => {
                self.injected.extend(Injection::click(egui::pos2(x, y)));
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
            Step::Theme(dark) => {
                self.dark = dark;
                theme::apply(ctx, dark);
                self.editors.set_dark(dark);
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
        serde_json::json!({
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
            },
            "goto": self.goto.as_ref().map(GotoFile::state),
            "find": { "open": self.find_open, "details": self.find.state() },
            "palette": self.palette.as_ref().map(|p| serde_json::json!({
                "query": p.query, "selected": p.selected,
                "matches": p.matches(&self.registry).iter().map(|id| self.registry.get(*id).key).collect::<Vec<_>>(),
            })),
            "dialog": self.dialog.as_ref().map(|d| match d {
                Dialog::CloseTab(p) => format!("close_tab:{}", p.display()),
                Dialog::Quit(p) => format!("quit:{}", p.len()),
            }),
            "status": self.status_model().to_json(),
            "tasks_busy": self.tracker.kinds(),
        })
    }
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
        self.scenario_step(&ctx);
        self.handle_close_request(&ctx);
        self.dispatch_shortcuts(&ctx);
        self.poll_disk(&ctx);
        self.update_title(&ctx);
        if self
            .message
            .as_ref()
            .is_some_and(|m| m.at.elapsed() > Duration::from_secs(12))
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
                status::show(ui, &p, &model);
            });

        actions.extend(self.draw_left(ui, &p));
        self.draw_bottom(ui, &p);
        self.draw_project_panel(ui, &p);
        self.draw_central(ui, &p);

        // Échap dans l'arbre : retour à l'éditeur.
        if self.tree.has_focus(&ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.focus_editor();
        }

        self.draw_overlays(&ctx, &p);
        if let Some(terminal) = &mut self.terminal {
            terminal.ui_global(ui);
        }
        self.draw_dialog(&ctx, &p);

        for id in actions {
            self.run_action(id);
        }
    }

    fn on_exit(&mut self) {
        self.tasks.cancel_all();
    }
}
