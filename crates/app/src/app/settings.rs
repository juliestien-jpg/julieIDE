//! Réglages côté application : surveillance des fichiers (un `SettingsWatcher` par
//! projet), redistribution à chaud aux services selon les sections changées,
//! fenêtre de réglages (écriture, sondes, « Parcourir… », ouverture du fichier),
//! écriture des réglages isolés (thème, méthode de mise à jour).
//!
//! Aucune règle de lecture ni d'écriture ici : tout est dans `julie-core`
//! (`SettingsFiles`, `SettingsWatcher`) ; ce fichier relie ces appels à l'interface.

use super::*;

use julie_core::settings::{Scope, SettingValue, SettingsChanged, SettingsProblem};
use julie_core::SettingsFiles;
use julie_core::SettingsWatcher;
use julie_phpunit::{ProbeEvent, ProbeRequest};

use crate::settingsui::{self, Page, ProbeAction, ProbeState, SettingsDialog, SettingsIntent};

/// Le thème réglé : `clair` donne le thème clair, tout le reste le sombre.
fn theme_is_dark(settings: &Settings) -> bool {
    settings
        .get(&["app", "theme"])
        .and_then(|v| v.as_str())
        .is_none_or(|v| v != "clair")
}

impl JulieStormApp {
    // ----- Surveillance ----------------------------------------------------------

    /// (Re)lance la surveillance pour ce projet (ou sans projet). Un bus à part par
    /// surveillant : les événements d'un ancien projet ne se mélangent pas aux
    /// nouveaux.
    pub(super) fn restart_settings_watcher(&mut self, project: Option<&julie_core::Project>) {
        let files = SettingsFiles::new(self.user_settings_file.clone(), project.cloned());
        let bus = Bus::new();
        let ctx = self.ctx.clone();
        bus.set_waker(move || ctx.request_repaint());
        self.settings_watcher = Some(SettingsWatcher::start(files.clone(), bus.publisher()));
        self.settings_bus = bus;
        self.settings_files = files;
        self.settings_first = true;
        self.settings_problems.clear();
    }

    pub(super) fn pump_settings(&mut self) {
        for event in self.settings_bus.drain() {
            if let Event::SettingsChanged(change) = event {
                self.on_settings_changed(change);
            }
        }
    }

    fn on_settings_changed(&mut self, change: SettingsChanged) {
        let first = std::mem::take(&mut self.settings_first);
        self.report_settings_problems(&change.problems);
        if first && change.settings == self.settings {
            return;
        }
        let old = std::mem::replace(&mut self.settings, change.settings.clone());
        let concerns = |section: &str| change.concerns(section);
        if concerns("app") {
            let dark = theme_is_dark(&self.settings);
            if dark != self.dark {
                self.set_dark(dark);
            }
        }
        if concerns("vcs") {
            self.redistribute_vcs(&old);
        }
        if concerns("index") {
            self.redistribute_index();
        }
        if concerns("phpunit") {
            self.redistribute_phpunit();
        }
        if concerns("terminal") {
            let settings: TerminalSettings = self.settings.section("terminal").unwrap_or_default();
            if let Some(terminal) = &mut self.terminal {
                terminal.apply_settings(&self.tasks, settings);
            }
        }
        self.ctx.request_repaint();
    }

    fn redistribute_vcs(&mut self, old: &Settings) {
        let new: VcsSettings = self.settings.section("vcs").unwrap_or_default();
        let before: VcsSettings = old.section("vcs").unwrap_or_default();
        self.update_method = new.update_method;
        let tasks = self.tasks.clone();
        let Some(project) = &mut self.project else {
            return;
        };
        if new.git_path != before.git_path {
            // L'exécutable n'est lu qu'à la construction : on rouvre le dépôt.
            project.vcs = Vcs::new(tasks, &new);
            project.repo = None;
            project.git = GitInfo::Unknown;
            project.vcs.open(project.project.root().to_path_buf());
        } else {
            project.vcs = project.vcs.clone().with_settings(new);
        }
    }

    fn redistribute_index(&mut self) {
        let mut settings: IndexSettings = self.settings.section("index").unwrap_or_default();
        // Toujours exclus, quoi que dise le réglage.
        for always in julie_core::project::ALWAYS_EXCLUDED.iter().rev() {
            if !settings.excluded.iter().any(|e| e == always) {
                settings.excluded.insert(0, (*always).to_owned());
            }
        }
        let tasks = self.tasks.clone();
        let Some(project) = &mut self.project else {
            return;
        };
        project.index.cancel_search();
        if let Some(id) = self.scan_task.take() {
            tasks.cancel(id);
        }
        let index = Arc::new(IndexService::new(tasks, project.project.clone(), settings));
        self.scan_task = Some(index.scan().id());
        project.index = index.clone();
        project.index_ready = false;
        self.tree.set_index(index);
        self.tree.refresh();
        self.find.refresh_now();
    }

    fn redistribute_phpunit(&mut self) {
        let settings = PhpUnitSettings::from_settings(&self.settings).unwrap_or_default();
        if let Some(project) = &self.project {
            self.tests.set_service(PhpUnit::new(
                self.tasks.clone(),
                project.project.clone(),
                settings,
            ));
        }
    }

    fn report_settings_problems(&mut self, problems: &[SettingsProblem]) {
        let known = |p: &SettingsProblem, list: &[SettingsProblem]| {
            list.iter()
                .any(|q| q.path == p.path && q.line == p.line && q.message == p.message)
        };
        let fresh: Vec<SettingsProblem> = problems
            .iter()
            .filter(|p| !known(p, &self.settings_problems))
            .cloned()
            .collect();
        self.settings_problems = problems.to_vec();
        if let Some(problem) = fresh.first() {
            let file = problem
                .path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let text = t::settings_unreadable(&file, problem.line, &problem.message);
            self.toast(
                text,
                true,
                Some(ToastAction::OpenFile(problem.path.clone(), problem.line)),
            );
        }
    }

    // ----- Écriture de réglages isolés -------------------------------------------

    /// Écrit (ou retire) une clé dans un fil ; le surveillant la prendra en compte.
    pub(super) fn write_setting(
        &mut self,
        scope: Scope,
        path: &'static [&'static str],
        value: Option<SettingValue>,
    ) {
        let files = self.settings_files.clone();
        let channel = self.channel.clone();
        std::thread::spawn(move || {
            let result = match value {
                Some(value) => files.set(scope, path, value),
                None => files.unset(scope, path),
            };
            channel.send(Msg::SettingsWritten {
                result: result.map_err(|e| e.to_string()),
            });
        });
    }

    pub(super) fn on_settings_written(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                if let Some(watcher) = &self.settings_watcher {
                    watcher.check_now();
                }
            }
            Err(error) => self.toast(
                format!("{}{error}", t::SETTINGS_WRITE_FAILED_PREFIX),
                true,
                None,
            ),
        }
    }

    // ----- La fenêtre -------------------------------------------------------------

    pub(super) fn open_settings(&mut self, page: Option<Page>) {
        self.goto = None;
        self.find_open = false;
        self.palette = None;
        self.ctx_menu = None;
        let has_user = self.settings_files.path(Scope::User).is_some();
        let has_project = self.project.is_some();
        let page = page.or(self.settings_page);
        let mut dialog = SettingsDialog::new(&self.settings, page, has_project, has_user);
        dialog.focus_filter();
        self.settings_dialog = Some(dialog);
    }

    fn close_settings(&mut self) {
        if let Some(dialog) = self.settings_dialog.take() {
            self.settings_page = Some(dialog.page);
        }
        self.focus_editor();
    }

    pub(super) fn draw_settings(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(mut dialog) = self.settings_dialog.take() else {
            return;
        };
        let root = self.project_root();
        let project_file = self.settings_files.path(Scope::Project);
        let user_file = self.settings_files.path(Scope::User);
        let intents = {
            let env = settingsui::Env {
                root: root.as_deref(),
                user_file: user_file.as_deref(),
                project_file: project_file.as_deref(),
            };
            dialog.show(ctx, p, &env)
        };
        self.settings_dialog = Some(dialog);
        for intent in intents {
            self.run_settings_intent(intent);
        }
    }

    pub(super) fn run_settings_intent(&mut self, intent: SettingsIntent) {
        match intent {
            SettingsIntent::Close => self.close_settings(),
            SettingsIntent::Apply { close, edits } => {
                if let Some(problem) = self
                    .settings_problems
                    .iter()
                    .find(|p| {
                        edits.iter().any(|(scope, _)| {
                            Some(&p.path) == self.settings_files.path(*scope).as_ref()
                        })
                    })
                    .cloned()
                {
                    // Jamais d'écrasement d'un fichier qu'on ne comprend pas.
                    let file = problem.path.display().to_string();
                    if let Some(dialog) = &mut self.settings_dialog {
                        dialog.failed(t::settings_refused(&file));
                    }
                    return;
                }
                let files = self.settings_files.clone();
                let channel = self.channel.clone();
                std::thread::spawn(move || {
                    let mut result = Ok(());
                    for (scope, list) in &edits {
                        if let Err(error) = files.apply(*scope, list) {
                            result = Err(match &error {
                                julie_core::Error::SettingsUnreadable { path, .. } => {
                                    t::settings_refused(&path.display().to_string())
                                }
                                other => other.to_string(),
                            });
                            break;
                        }
                    }
                    channel.send(Msg::SettingsApplied { close, result });
                });
            }
            SettingsIntent::OpenFile(scope) => {
                let files = self.settings_files.clone();
                let channel = self.channel.clone();
                std::thread::spawn(move || match files.ensure_file(scope) {
                    Ok(path) => channel.send(Msg::OpenPath { path, line: None }),
                    Err(error) => channel.send(Msg::Notice {
                        text: format!("{}{error}", t::SETTINGS_WRITE_FAILED_PREFIX),
                        error: true,
                    }),
                });
                self.close_settings();
            }
            SettingsIntent::Probe {
                field,
                token,
                action,
            } => self.start_probe(field, token, action),
            SettingsIntent::Browse { field, scope } => {
                let start = self.project_root();
                let channel = self.channel.clone();
                std::thread::spawn(move || {
                    let mut dialog = rfd::FileDialog::new();
                    if let Some(start) = start {
                        dialog = dialog.set_directory(start);
                    }
                    if let Some(path) = dialog.pick_file() {
                        channel.send(Msg::SettingsBrowsed { field, scope, path });
                    }
                });
            }
        }
    }

    pub(super) fn on_settings_applied(&mut self, close: bool, result: Result<(), String>) {
        let Some(dialog) = &mut self.settings_dialog else {
            // Fermée entre-temps : l'écriture a eu lieu, le surveillant s'en charge.
            if result.is_ok() {
                self.on_settings_written(Ok(()));
            }
            return;
        };
        match result {
            Ok(()) => {
                dialog.committed();
                if let Some(watcher) = &self.settings_watcher {
                    watcher.check_now();
                }
                if close {
                    self.close_settings();
                }
            }
            Err(error) => dialog.failed(error),
        }
    }

    fn start_probe(&mut self, field: &'static str, token: u64, action: ProbeAction) {
        let root = self.project_root().unwrap_or_else(std::env::temp_dir);
        let tasks = self.tasks.clone();
        let channel = self.channel.clone();
        let Some(dialog) = &mut self.settings_dialog else {
            return;
        };
        if let Some(old) = dialog.probes.get(field).and_then(|s| s.task) {
            tasks.cancel(old);
        }
        match action {
            ProbeAction::Php(request) => {
                let handle = julie_phpunit::probe::spawn(&tasks, root, request);
                if let Some(slot) = dialog.probes.get_mut(field) {
                    slot.task = Some(handle.id());
                }
            }
            ProbeAction::Git(path) => {
                std::thread::spawn(move || {
                    channel.send(Msg::SettingsProbe {
                        field,
                        token,
                        state: probe_git(&path),
                    });
                });
            }
            ProbeAction::Exists(path) => {
                std::thread::spawn(move || {
                    let state = if path.is_dir() {
                        ProbeState::Done(1, t::PROBE_IS_DIR.to_owned())
                    } else if path.is_file() {
                        ProbeState::Done(0, t::PROBE_EXISTS.to_owned())
                    } else {
                        ProbeState::Done(1, t::PROBE_NOT_EXISTS.to_owned())
                    };
                    channel.send(Msg::SettingsProbe {
                        field,
                        token,
                        state,
                    });
                });
            }
        }
    }

    /// Une sonde PHP / PHPUnit / configuration est arrivée par le bus.
    pub(super) fn on_probe_event(&mut self, task: Option<TaskId>, event: &ProbeEvent) {
        let (Some(task), Some(dialog)) = (task, &mut self.settings_dialog) else {
            return;
        };
        let (level, text) = t::probe_text(&event.result);
        dialog.probe_by_task(task, ProbeState::Done(level, text));
        let _: &ProbeRequest = &event.request;
    }

    // ----- Mode scénario ---------------------------------------------------------

    pub(super) fn settings_state(&self) -> serde_json::Value {
        let project_file = self.settings_files.path(Scope::Project);
        let user_file = self.settings_files.path(Scope::User);
        let root = self.project_root();
        let env = settingsui::Env {
            root: root.as_deref(),
            user_file: user_file.as_deref(),
            project_file: project_file.as_deref(),
        };
        serde_json::json!({
            "dialog": self.settings_dialog.as_ref().map(|d| d.state(&env)),
            "files": {
                "user": user_file.map(|p| p.display().to_string()),
                "project": project_file.map(|p| p.display().to_string()),
            },
            "problems": self.settings_problems.iter().map(|p| serde_json::json!({
                "file": p.path.display().to_string(), "line": p.line, "message": p.message,
            })).collect::<Vec<_>>(),
            "effective": {
                "dark": theme_is_dark(&self.settings),
                "phpunit": PhpUnitSettings::from_settings(&self.settings).ok().map(|s| serde_json::json!({
                    "php_path": s.php_path, "phpunit_path": s.phpunit_path,
                    "configuration": s.configuration, "extra_args": s.extra_args, "env": s.env,
                })),
                "update_method": self.update_method.map(|m| format!("{m:?}")),
                "index_excluded": self.settings.section::<IndexSettings>("index").ok().map(|s| s.excluded),
            },
        })
    }

    /// Les commandes `ui settings.*`. `None` : ce n'est pas une commande de réglages.
    pub(super) fn ui_command_settings(
        &mut self,
        command: &str,
        arg: &str,
    ) -> Option<Result<(), String>> {
        let name = command.strip_prefix("settings.")?;
        Some(self.settings_command(name, arg))
    }

    fn settings_command(&mut self, name: &str, arg: &str) -> Result<(), String> {
        let text = arg.replace("\\n", "\n");
        if name == "open" {
            let page = if arg.is_empty() {
                None
            } else {
                Some(
                    Page::parse(arg)
                        .ok_or("page inconnue : phpunit, git, terminal, search, appearance")?,
                )
            };
            self.open_settings(page);
            return Ok(());
        }
        let dialog = self
            .settings_dialog
            .as_mut()
            .ok_or("la fenêtre de réglages n'est pas ouverte")?;
        match name {
            "page" => {
                dialog.page = Page::parse(arg).ok_or("page inconnue")?;
            }
            "scope" => {
                let scope = match arg {
                    "user" | "tous" => Scope::User,
                    "project" | "projet" => Scope::Project,
                    _ => return Err("`settings.scope user|project`".to_owned()),
                };
                dialog.set_scope(scope)?;
            }
            "filter" => dialog.filter = arg.to_owned(),
            "set" => {
                let (key, value) = text.split_once(char::is_whitespace).unwrap_or((&text, ""));
                let scope = dialog.scope();
                dialog.set_text(key, scope, value.trim_start())?;
            }
            "unset" => {
                let scope = dialog.scope();
                dialog.unset(arg, scope)?;
            }
            "env" => {
                let scope = dialog.scope();
                match arg.strip_prefix('-') {
                    Some(name) => dialog.unset_env(scope, name.trim()),
                    None => {
                        let (n, v) = arg.split_once('=').ok_or("`settings.env NOM=valeur`")?;
                        dialog.set_env(scope, n.trim(), v);
                    }
                }
            }
            "apply" | "ok" => {
                let intents = dialog.request_apply(name == "ok");
                if intents.is_empty() {
                    if let Some((_, message)) = &dialog.message {
                        return Err(message.clone());
                    }
                }
                for intent in intents {
                    self.run_settings_intent(intent);
                }
            }
            "cancel" => self.close_settings(),
            "open-file" => {
                let scope = dialog.scope();
                self.run_settings_intent(SettingsIntent::OpenFile(scope));
            }
            other => return Err(format!("commande de réglages inconnue : « {other} »")),
        }
        Ok(())
    }
}

/// `git --version` (ou le chemin réglé), 5 s au plus.
fn probe_git(path: &Path) -> ProbeState {
    use std::io::Read;
    use std::process::{Command, Stdio};
    if path.is_dir() {
        return ProbeState::Done(1, t::PROBE_IS_DIR.to_owned());
    }
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ProbeState::Done(1, t::PROBE_NOT_EXISTS.to_owned());
        }
        Err(error) => return ProbeState::Done(1, error.to_string()),
    };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > Duration::from_secs(5) => {
                let _ = child.kill();
                let _ = child.wait();
                return ProbeState::Done(1, t::PROBE_NO_ANSWER.to_owned());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => return ProbeState::Done(1, error.to_string()),
        }
    }
    let mut out = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut out);
    }
    let first = out.lines().next().unwrap_or_default().trim().to_owned();
    if first.starts_with("git version") {
        ProbeState::Done(0, first)
    } else {
        ProbeState::Done(1, t::probe_not_git(&first))
    }
}
