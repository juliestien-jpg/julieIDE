//! Branchement minimal du terminal dans la coquille : onglets de sessions dans
//! la fenêtre d'outils du bas. dev-interface en reprendra l'habillage ; tout
//! ce que la coquille appelle est ici (voir `mod.rs`).

use std::path::{Path, PathBuf};

use eframe::egui::{self, Id, Key, KeyboardShortcut, Modifiers, Ui};
use julie_core::Tasks;
use julie_terminal::{
    Profile, ProfileKind, Profiles, Session, SessionId, SessionOptions, TerminalEvent,
    TerminalSettings, Terminals,
};

use super::view::{TerminalIntent, TerminalView};
use crate::target::{MenuRequest, Target, TermRef};

const NEW_TAB: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::T);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::F4);
const TITLE_MAX_CHARS: usize = 32;

/// Textes de l'onglet Terminal (à reprendre dans le catalogue de `redacteur`).
mod texts {
    use julie_terminal::{ProfileKind, TerminalError};
    use std::path::Path;

    pub const NEW_TAB_TOOLTIP: &str = "Nouveau terminal (Ctrl+Maj+T)";
    pub const CHOOSE_SHELL_TOOLTIP: &str = "Choisir l'interpréteur";
    pub const CLOSE_TAB_TOOLTIP: &str = "Fermer (Ctrl+F4)";
    pub const RENAME_TOOLTIP: &str = "Double-clic : renommer";
    pub const NO_TERMINAL: &str = "Aucun terminal ouvert.";
    pub const DETECTING: &str = "Recherche des interpréteurs…";
    pub const RELAUNCH: &str = "Relancer";
    pub const CONFIRM_CLOSE_TAB: &str = "Un programme tourne encore dans ce terminal";
    pub const CONFIRM_EXIT: &str = "Un programme tourne encore dans un terminal";
    pub const CONFIRM_QUESTION: &str = "Le fermer ?";
    pub const CONFIRM_YES: &str = "Fermer";
    pub const CONFIRM_NO: &str = "Annuler";

    pub fn shell_name(kind: ProfileKind) -> &'static str {
        match kind {
            ProfileKind::Cmd => "cmd",
            ProfileKind::GitBash => "Git Bash",
            ProfileKind::PowerShell => "PowerShell",
        }
    }

    pub fn shell_missing(kind: ProfileKind, path: &Path) -> String {
        format!("{} introuvable à {}", shell_name(kind), path.display())
    }

    pub fn exited(code: u32) -> String {
        format!("Processus terminé (code {code})")
    }

    pub fn failed(error: &TerminalError) -> String {
        match error {
            TerminalError::ShellNotFound(path) => {
                format!("Interpréteur introuvable : {}", path.display())
            }
            TerminalError::DirectoryNotFound(path) => {
                format!("Dossier de départ introuvable : {}", path.display())
            }
            other => format!("Le terminal n'a pas pu démarrer ({other})"),
        }
    }

    pub fn running(programs: &[String]) -> String {
        format!("({})", programs.join(", "))
    }
}

enum Ended {
    Exited(u32),
    Failed(String),
}

struct Tab {
    session: Session,
    view: TerminalView,
    /// « cmd », « Git Bash (2) »…
    name: String,
    /// Nom donné par l'utilisatrice (double-clic).
    custom_name: Option<String>,
    /// Titre fixé par le shell (séquence OSC).
    title: Option<String>,
    ended: Option<Ended>,
}

impl Tab {
    fn label(&self) -> String {
        if let Some(name) = &self.custom_name {
            return name.clone();
        }
        match &self.title {
            Some(title) => {
                if title.chars().count() > TITLE_MAX_CHARS {
                    let tail: String = title
                        .chars()
                        .rev()
                        .take(TITLE_MAX_CHARS - 1)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    format!("…{tail}")
                } else {
                    title.clone()
                }
            }
            None => self.name.clone(),
        }
    }
}

enum Confirm {
    CloseTab(SessionId, Vec<String>),
    Exit(Vec<String>),
}

/// L'onglet « Terminal » de la fenêtre d'outils du bas.
pub struct TerminalPanel {
    terminals: Terminals,
    cwd: PathBuf,
    profiles: Option<Profiles>,
    tabs: Vec<Tab>,
    active: usize,
    opened: Vec<(ProfileKind, u32)>,
    /// Le premier terminal s'ouvre tout seul à la première visite.
    auto_opened: bool,
    focus_after_open: bool,
    confirm: Option<Confirm>,
    exit_confirmed: bool,
    renaming: Option<(SessionId, String)>,
    clipboard: Option<String>,
    /// Menu contextuel demandé à cette image (clic droit sur l'écran ou sur un onglet).
    pub menu_request: Option<MenuRequest>,
}

impl TerminalPanel {
    /// `cwd` : dossier de départ des shells (racine du projet).
    pub fn new(tasks: Tasks, cwd: PathBuf, settings: TerminalSettings) -> Self {
        let terminals = Terminals::new(tasks, settings);
        terminals.detect_profiles();
        Self {
            terminals,
            cwd,
            profiles: None,
            tabs: Vec::new(),
            active: 0,
            opened: Vec::new(),
            auto_opened: false,
            focus_after_open: false,
            confirm: None,
            exit_confirmed: false,
            renaming: None,
            clipboard: None,
            menu_request: None,
        }
    }

    // ----- Ce que le menu contextuel demande ---------------------------------------

    pub fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn has_selection(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|tab| tab.session.has_selection())
    }

    pub fn selection_text(&self) -> Option<String> {
        self.tabs.get(self.active)?.session.selection_text()
    }

    /// Le texte arrive en `Event::Paste` au terminal qui a le focus.
    pub fn paste(&mut self, ctx: &egui::Context) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.view.request_focus();
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
    }

    pub fn select_all(&mut self) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.view.select_all(&tab.session);
        }
    }

    pub fn clear_history(&mut self) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.session.clear_history();
        }
    }

    pub fn begin_rename(&mut self, index: usize) {
        if let Some(tab) = self.tabs.get(index) {
            self.active = index;
            self.renaming = Some((tab.session.id(), tab.label()));
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.request_close(index);
        }
    }

    /// Ouvre un onglet de ce type d'interpréteur (s'il est installé).
    pub fn open_kind(&mut self, kind: ProfileKind) {
        let profile = self
            .profiles
            .as_ref()
            .and_then(|profiles| profiles.get(kind))
            .filter(|p| p.available)
            .cloned();
        if let Some(profile) = profile {
            self.open(profile);
        }
    }

    /// Pourquoi cet interpréteur ne peut pas être ouvert (infobulle de l'entrée grisée).
    pub fn profile_problem(&self, kind: ProfileKind) -> Option<String> {
        let profiles = self.profiles.as_ref()?;
        let profile = profiles.get(kind)?;
        (!profile.available).then(|| texts::shell_missing(kind, &profile.program))
    }

    pub fn tab_names(&self) -> Vec<String> {
        self.tabs.iter().map(Tab::label).collect()
    }

    /// De nouveaux réglages `[terminal]` : les **prochains** onglets les suivent ;
    /// les sessions ouvertes ne changent pas.
    pub fn apply_settings(&mut self, tasks: &Tasks, settings: TerminalSettings) {
        tasks.spawn(
            julie_core::Domain::Terminal,
            "terminal.profiles",
            move |ctx| {
                ctx.publish(TerminalEvent::Profiles(julie_terminal::detect_profiles(
                    &settings,
                )));
                Ok(())
            },
        );
    }

    /// Le terminal actif a le focus clavier (la coquille lui laisse alors
    /// Ctrl+S et les autres touches du shell).
    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|tab| tab.view.has_focus(ctx))
    }

    /// Donne le focus au terminal actif (Alt+F12) ; en ouvre un s'il n'y en a pas.
    pub fn focus(&mut self) {
        match self.tabs.get_mut(self.active) {
            Some(tab) => tab.view.request_focus(),
            None => self.focus_after_open = true,
        }
    }

    pub fn on_event(&mut self, event: &TerminalEvent) {
        match event {
            TerminalEvent::Profiles(profiles) => self.profiles = Some(profiles.clone()),
            TerminalEvent::TitleChanged { session, title } => {
                if let Some(tab) = self.tab_mut(*session) {
                    // Le titre par défaut d'une console est le chemin du programme.
                    let default = tab.session.profile().program.as_os_str().to_string_lossy();
                    tab.title = title
                        .clone()
                        .filter(|t| !t.trim().is_empty() && !t.eq_ignore_ascii_case(&default));
                }
            }
            TerminalEvent::Exited { session, code: 0 } => {
                if let Some(index) = self.index_of(*session) {
                    self.remove_tab(index);
                }
            }
            TerminalEvent::Exited { session, code } => {
                if let Some(tab) = self.tab_mut(*session) {
                    tab.ended = Some(Ended::Exited(*code));
                }
            }
            TerminalEvent::Failed { session, error } => {
                if let Some(tab) = self.tab_mut(*session) {
                    tab.ended = Some(Ended::Failed(texts::failed(error)));
                }
            }
            TerminalEvent::Clipboard { text, .. } => self.clipboard = Some(text.clone()),
            TerminalEvent::Started { .. }
            | TerminalEvent::Output { .. }
            | TerminalEvent::Bell { .. }
            | TerminalEvent::Closed { .. } => {}
        }
    }

    fn index_of(&self, id: SessionId) -> Option<usize> {
        self.tabs.iter().position(|t| t.session.id() == id)
    }

    fn tab_mut(&mut self, id: SessionId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.session.id() == id)
    }

    fn remove_tab(&mut self, index: usize) {
        // Lâcher la session la ferme (tout l'arbre), sans attendre.
        let had_focus = index == self.active;
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if index < self.active {
            self.active -= 1;
        }
        if had_focus {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                tab.view.request_focus();
            }
        }
    }

    fn open(&mut self, profile: Profile) {
        let kind = profile.kind;
        let count = match self.opened.iter_mut().find(|(k, _)| *k == kind) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                self.opened.push((kind, 1));
                1
            }
        };
        let base = texts::shell_name(kind);
        let name = if count > 1 {
            format!("{base} ({count})")
        } else {
            base.to_string()
        };
        let session = self
            .terminals
            .open(SessionOptions::new(profile, self.cwd.clone()));
        let mut view = TerminalView::new(session.id());
        view.request_focus();
        self.tabs.push(Tab {
            session,
            view,
            name,
            custom_name: None,
            title: None,
            ended: None,
        });
        self.active = self.tabs.len() - 1;
    }

    /// Ouvre un onglet du terminal par défaut dans ce dossier (menu contextuel
    /// « Ouvrir un terminal ici », ajout de dev-interface).
    pub fn open_default_in(&mut self, dir: PathBuf) {
        let saved = std::mem::replace(&mut self.cwd, dir);
        self.open_default();
        self.cwd = saved;
    }

    fn open_default(&mut self) {
        if let Some(profile) = self
            .profiles
            .as_ref()
            .and_then(|p| p.default_profile())
            .cloned()
        {
            self.open(profile);
        }
    }

    fn relaunch(&mut self, index: usize) {
        let tab = &mut self.tabs[index];
        let profile = tab.session.profile().clone();
        tab.session = self
            .terminals
            .open(SessionOptions::new(profile, self.cwd.clone()));
        tab.view = TerminalView::new(tab.session.id());
        tab.view.request_focus();
        tab.title = None;
        tab.ended = None;
    }

    /// Fermer un onglet : confirmation si un programme y tourne encore.
    fn request_close(&mut self, index: usize) {
        let tab = &self.tabs[index];
        let running = tab.session.running_programs();
        if running.is_empty() {
            self.remove_tab(index);
        } else {
            self.confirm = Some(Confirm::CloseTab(tab.session.id(), running));
        }
    }

    /// À appeler à chaque image, que l'onglet Terminal soit affiché ou non :
    /// confirmation à la fermeture de l'IDE, presse-papiers demandé par un
    /// programme (OSC 52).
    pub fn ui_global(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if let Some(text) = self.clipboard.take() {
            ctx.copy_text(text);
        }
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && !self.exit_confirmed && self.confirm.is_none() {
            let running: Vec<String> = self
                .tabs
                .iter()
                .flat_map(|t| t.session.running_programs())
                .fold(Vec::new(), |mut all, name| {
                    if !all.contains(&name) {
                        all.push(name);
                    }
                    all
                });
            if !running.is_empty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(Confirm::Exit(running));
            }
        }

        let Some(confirm) = &self.confirm else {
            return;
        };
        let (message, programs) = match confirm {
            Confirm::CloseTab(_, programs) => (texts::CONFIRM_CLOSE_TAB, programs),
            Confirm::Exit(programs) => (texts::CONFIRM_EXIT, programs),
        };
        let mut answer = None;
        let modal = egui::Modal::new(Id::unique("julie-terminal-confirm")).show(&ctx, |ui| {
            ui.label(format!("{message} {}.", texts::running(programs)));
            ui.label(texts::CONFIRM_QUESTION);
            ui.horizontal(|ui| {
                if ui.button(texts::CONFIRM_YES).clicked() {
                    answer = Some(true);
                }
                if ui.button(texts::CONFIRM_NO).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match (answer, self.confirm.take()) {
            (Some(true), Some(Confirm::CloseTab(id, _))) => {
                if let Some(index) = self.index_of(id) {
                    self.remove_tab(index);
                }
            }
            (Some(true), Some(Confirm::Exit(_))) => {
                self.exit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Some(_), _) => {}
            (None, confirm) => self.confirm = confirm,
        }
    }

    /// Le contenu de l'onglet Terminal. Rend les demandes d'ouverture de fichier.
    pub fn show(&mut self, ui: &mut Ui) -> Vec<TerminalIntent> {
        if !self.auto_opened && self.profiles.is_some() {
            self.auto_opened = true;
            if self.tabs.is_empty() {
                self.open_default();
            }
        }
        if self.focus_after_open {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                self.focus_after_open = false;
                tab.view.request_focus();
            }
        }

        // Raccourcis du contexte Terminal.
        let focused = self.has_focus(ui.ctx());
        if focused {
            let (new_tab, close_tab) = ui
                .ctx()
                .input_mut(|i| (i.consume_shortcut(&NEW_TAB), i.consume_shortcut(&CLOSE_TAB)));
            if new_tab {
                self.open_default();
            }
            if close_tab && !self.tabs.is_empty() {
                self.request_close(self.active);
            }
        }

        self.tab_bar(ui);
        ui.separator();

        let Some(tab) = self.tabs.get_mut(self.active) else {
            ui.weak(if self.profiles.is_none() {
                texts::DETECTING
            } else {
                texts::NO_TERMINAL
            });
            return Vec::new();
        };

        let mut relaunch = false;
        match &tab.ended {
            Some(Ended::Failed(message)) => {
                ui.colored_label(ui.visuals().warn_fg_color, message);
                relaunch = ui.button(texts::RELAUNCH).clicked();
                if relaunch {
                    self.relaunch(self.active);
                }
                return Vec::new();
            }
            Some(Ended::Exited(code)) => {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().warn_fg_color, texts::exited(*code));
                    relaunch = ui.button(texts::RELAUNCH).clicked();
                });
            }
            None => {}
        }
        let output = tab.view.show(ui, &tab.session);
        if let Some(pos) = tab.view.take_menu_pos() {
            self.menu_request = Some(MenuRequest {
                target: Target::Terminal(TermRef {
                    has_selection: tab.session.has_selection(),
                    tab: self.active,
                    on_tab: false,
                }),
                pos,
                by_keyboard: false,
            });
        }
        if relaunch {
            self.relaunch(self.active);
        }
        output.intents
    }

    fn tab_bar(&mut self, ui: &mut Ui) {
        let mut select = None;
        let mut close = None;
        let mut open = None;
        let mut new_default = false;
        let mut menu: Option<(usize, egui::Pos2)> = None;
        ui.horizontal(|ui| {
            for (index, tab) in self.tabs.iter().enumerate() {
                let id = tab.session.id();
                if let Some((renaming, text)) = &mut self.renaming {
                    if *renaming == id {
                        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                        edit.request_focus();
                        let (enter, escape) =
                            ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                        if enter || escape || edit.lost_focus() {
                            let name = text.trim().to_string();
                            let commit = !escape && !name.is_empty();
                            self.renaming = None;
                            if commit {
                                select = Some((index, Some(name)));
                            }
                        }
                        continue;
                    }
                }
                let label = ui
                    .selectable_label(index == self.active, tab.label())
                    .on_hover_text(texts::RENAME_TOOLTIP);
                if label.double_clicked() {
                    self.renaming = Some((id, tab.label()));
                } else if label.clicked() {
                    select = Some((index, None));
                }
                if label.secondary_clicked() {
                    select = Some((index, None));
                    menu = Some((
                        index,
                        label
                            .interact_pointer_pos()
                            .unwrap_or(label.rect.left_bottom()),
                    ));
                }
                if ui
                    .small_button("×")
                    .on_hover_text(texts::CLOSE_TAB_TOOLTIP)
                    .clicked()
                {
                    close = Some(index);
                }
            }
            if ui
                .small_button("+")
                .on_hover_text(texts::NEW_TAB_TOOLTIP)
                .clicked()
            {
                new_default = true;
            }
            ui.menu_button("▾", |ui| {
                let Some(profiles) = &self.profiles else {
                    ui.weak(texts::DETECTING);
                    return;
                };
                for profile in &profiles.profiles {
                    // PowerShell est hors périmètre v0 : proposé seulement s'il
                    // est installé.
                    if profile.kind == ProfileKind::PowerShell && !profile.available {
                        continue;
                    }
                    let button = ui
                        .add_enabled(
                            profile.available,
                            egui::Button::new(texts::shell_name(profile.kind)),
                        )
                        .on_disabled_hover_text(texts::shell_missing(
                            profile.kind,
                            &profile.program,
                        ));
                    if button.clicked() {
                        open = Some(profile.clone());
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(texts::CHOOSE_SHELL_TOOLTIP);
        });

        if let Some((index, pos)) = menu {
            self.menu_request = Some(MenuRequest {
                target: Target::Terminal(TermRef {
                    has_selection: self.tabs[index].session.has_selection(),
                    tab: index,
                    on_tab: true,
                }),
                pos,
                by_keyboard: false,
            });
        }
        if let Some((index, rename)) = select {
            if let Some(name) = rename {
                self.tabs[index].custom_name = Some(name);
            }
            self.active = index;
            self.tabs[index].view.request_focus();
        }
        if let Some(index) = close {
            self.request_close(index);
        }
        if new_default {
            self.open_default();
        }
        if let Some(profile) = open {
            self.open(profile);
        }
    }

    /// Ferme tous les terminaux **sans attendre** : chaque arbre de processus
    /// est tué tout de suite (`TerminateJobObject`, appel qui ne bloque pas),
    /// la fermeture des pseudo-consoles se termine dans les fils des sessions.
    /// Si l'IDE se termine avant, Windows ferme les jobs (`KILL_ON_JOB_CLOSE`)
    /// et les conhost s'arrêtent d'eux-mêmes : aucun orphelin.
    pub fn close_all(&mut self) {
        for tab in self.tabs.drain(..) {
            tab.session.close();
        }
        self.active = 0;
    }

    /// Ancien nom de [`Self::close_all`], gardé pour la coquille : ne bloque
    /// plus, `_timeout` est ignoré.
    pub fn shutdown(&mut self, _timeout: std::time::Duration) {
        self.close_all();
    }
}

/// Chemin absolu d'un `TerminalIntent::OpenFile` : les chemins MSYS
/// (`/c/Users/…`) redeviennent des chemins Windows ; un chemin relatif l'est au
/// dossier de départ des shells.
pub fn resolve_path(cwd: &Path, path: &str) -> PathBuf {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b'/' {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        return PathBuf::from(format!("{drive}:\\{}", path[3..].replace('/', "\\")));
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(path.replace('/', "\\"))
    }
}

impl TerminalPanel {
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
}

impl Drop for TerminalPanel {
    fn drop(&mut self) {
        // Fermeture de l'IDE : arbres tués sans attendre (voir `close_all`).
        self.close_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fermer l'IDE avec des terminaux occupés ne fige pas l'interface : le
    /// `Drop` du panneau rend la main tout de suite (la mort effective des
    /// arbres est vérifiée dans `julie-terminal`, tests `sessions`).
    #[cfg(windows)]
    #[test]
    fn dropping_the_panel_does_not_block() {
        use std::time::{Duration, Instant};
        let bus = julie_core::bus::Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let settings = TerminalSettings::default();
        let mut panel = TerminalPanel::new(tasks, std::env::temp_dir(), settings.clone());
        for _ in 0..3 {
            panel.open(julie_terminal::profile(ProfileKind::Cmd, &settings));
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        let started = |panel: &TerminalPanel| {
            panel
                .tabs
                .iter()
                .all(|t| t.session.process_ids().len() >= 2)
        };
        for tab in &panel.tabs {
            tab.session
                .send_text("%SystemRoot%\\System32\\PING.EXE -t 127.0.0.1\r");
        }
        while !started(&panel) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(started(&panel), "ping attendu dans chaque terminal");
        let before = Instant::now();
        drop(panel);
        let took = before.elapsed();
        assert!(took < Duration::from_millis(100), "Drop a bloqué {took:?}");
    }

    #[test]
    fn msys_and_relative_paths_are_resolved() {
        let cwd = Path::new(r"C:\projet");
        assert_eq!(
            resolve_path(cwd, "/c/Users/x/a.php"),
            PathBuf::from(r"C:\Users\x\a.php")
        );
        assert_eq!(
            resolve_path(cwd, "src/Foo.php"),
            PathBuf::from(r"C:\projet\src\Foo.php")
        );
        assert_eq!(
            resolve_path(cwd, r"D:\autre\b.js"),
            PathBuf::from(r"D:\autre\b.js")
        );
    }
}
