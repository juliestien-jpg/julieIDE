//! Registre d'actions : chaque commande de l'IDE a un identifiant stable (clé
//! `groupe.nom`, utilisée aussi par le mode scénario), un libellé du catalogue de
//! textes et un raccourci modifiable. Les menus, la palette d'actions et la
//! distribution des raccourcis clavier s'en nourrissent ; rien n'est codé en
//! dur ailleurs.
//!
//! Raccourcis : table de `docs/specs/raccourcis.md`. Les lignes marquées « nôtre »
//! dans la spécification, ou absentes de la table, sont signalées ci-dessous.

use eframe::egui::{Key, KeyboardShortcut, Modifiers};

use crate::texts as t;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionId {
    OpenFolder,
    SaveAll,
    CloseTab,
    ReloadFromDisk,
    Quit,
    GotoFile,
    FindInFiles,
    ActionPalette,
    PreviousTab,
    NextTab,
    LastTab,
    ToggleProject,
    ToggleGit,
    ToggleTests,
    ToggleSearch,
    ToggleTerminal,
    ToggleTheme,
    ToggleCommit,
    BottomTall,
    GitCommit,
    GitRefresh,
    TestsRun,
    TestsStop,
    TestsRerun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    Navigate,
    View,
    Git,
    Tests,
}

#[derive(Debug, Clone)]
pub struct Action {
    pub id: ActionId,
    /// Clé stable (`file.save_all`).
    pub key: &'static str,
    pub label: &'static str,
    pub menu: Option<Menu>,
    pub shortcut: Option<KeyboardShortcut>,
    /// Reste à l'IDE quand le terminal a le focus (le reste va au shell).
    pub in_terminal: bool,
}

pub struct Registry {
    actions: Vec<Action>,
}

const CTRL: Modifiers = Modifiers::CTRL;
const ALT: Modifiers = Modifiers::ALT;
const CTRL_SHIFT: Modifiers = Modifiers::CTRL.plus(Modifiers::SHIFT);
const CTRL_ALT: Modifiers = Modifiers::CTRL.plus(Modifiers::ALT);

fn sc(modifiers: Modifiers, key: Key) -> Option<KeyboardShortcut> {
    Some(KeyboardShortcut::new(modifiers, key))
}

impl Registry {
    pub fn new() -> Self {
        let a = |id, key, label, menu, shortcut, in_terminal| Action {
            id,
            key,
            label,
            menu,
            shortcut,
            in_terminal,
        };
        use ActionId::*;
        Self {
            actions: vec![
                // Nôtre : Ctrl+Maj+O (absent de la table de raccourcis.md).
                a(
                    OpenFolder,
                    "file.open_folder",
                    t::ACTION_OPEN_FOLDER,
                    Some(Menu::File),
                    sc(CTRL_SHIFT, Key::O),
                    true,
                ),
                a(
                    SaveAll,
                    "file.save_all",
                    t::ACTION_SAVE_ALL,
                    Some(Menu::File),
                    sc(CTRL, Key::S),
                    false,
                ),
                a(
                    CloseTab,
                    "file.close_tab",
                    t::ACTION_CLOSE_TAB,
                    Some(Menu::File),
                    sc(CTRL, Key::F4),
                    false,
                ),
                a(
                    ReloadFromDisk,
                    "file.reload_from_disk",
                    t::ACTION_RELOAD_DISK,
                    Some(Menu::File),
                    sc(CTRL_ALT, Key::Y),
                    false,
                ),
                a(
                    Quit,
                    "file.quit",
                    t::ACTION_QUIT,
                    Some(Menu::File),
                    None,
                    false,
                ),
                a(
                    GotoFile,
                    "nav.goto_file",
                    t::ACTION_GOTO_FILE,
                    Some(Menu::Navigate),
                    sc(CTRL_SHIFT, Key::N),
                    true,
                ),
                a(
                    FindInFiles,
                    "nav.find_in_files",
                    t::ACTION_FIND_IN_FILES,
                    Some(Menu::Navigate),
                    sc(CTRL_SHIFT, Key::F),
                    true,
                ),
                // Nôtre : la palette d'actions est « hors v0 » dans raccourcis.md.
                a(
                    ActionPalette,
                    "nav.action_palette",
                    t::ACTION_PALETTE,
                    Some(Menu::Navigate),
                    sc(CTRL_SHIFT, Key::A),
                    true,
                ),
                a(
                    PreviousTab,
                    "nav.previous_tab",
                    t::ACTION_PREVIOUS_TAB,
                    Some(Menu::Navigate),
                    sc(ALT, Key::ArrowLeft),
                    false,
                ),
                a(
                    NextTab,
                    "nav.next_tab",
                    t::ACTION_NEXT_TAB,
                    Some(Menu::Navigate),
                    sc(ALT, Key::ArrowRight),
                    false,
                ),
                a(
                    LastTab,
                    "nav.last_tab",
                    t::ACTION_LAST_TAB,
                    Some(Menu::Navigate),
                    sc(CTRL, Key::Tab),
                    false,
                ),
                a(
                    ToggleProject,
                    "view.project",
                    t::ACTION_TOGGLE_PROJECT,
                    Some(Menu::View),
                    sc(ALT, Key::Num1),
                    true,
                ),
                a(
                    ToggleGit,
                    "view.git",
                    t::ACTION_TOGGLE_GIT,
                    Some(Menu::View),
                    sc(ALT, Key::Num9),
                    true,
                ),
                a(
                    ToggleTests,
                    "view.tests",
                    t::ACTION_TOGGLE_TESTS,
                    Some(Menu::View),
                    sc(ALT, Key::Num4),
                    true,
                ),
                // Nôtre : Alt+3 (absent de la table).
                a(
                    ToggleSearch,
                    "view.search",
                    t::ACTION_TOGGLE_SEARCH,
                    Some(Menu::View),
                    sc(ALT, Key::Num3),
                    true,
                ),
                a(
                    ToggleTerminal,
                    "view.terminal",
                    t::ACTION_TOGGLE_TERMINAL,
                    Some(Menu::View),
                    sc(ALT, Key::F12),
                    true,
                ),
                a(
                    ToggleTheme,
                    "view.toggle_theme",
                    t::ACTION_TOGGLE_THEME,
                    Some(Menu::View),
                    None,
                    false,
                ),
                a(
                    ToggleCommit,
                    "view.commit",
                    t::ACTION_TOGGLE_COMMIT,
                    Some(Menu::View),
                    sc(ALT, Key::Num0),
                    true,
                ),
                // Nôtre : pas de raccourci (absent de la table).
                a(
                    BottomTall,
                    "view.bottom_tall",
                    t::ACTION_BOTTOM_TALL,
                    Some(Menu::View),
                    None,
                    false,
                ),
                a(
                    GitCommit,
                    "vcs.commit",
                    t::ACTION_GIT_COMMIT,
                    Some(Menu::Git),
                    sc(CTRL, Key::K),
                    false,
                ),
                a(
                    GitRefresh,
                    "vcs.refresh",
                    t::ACTION_GIT_REFRESH,
                    Some(Menu::Git),
                    None,
                    false,
                ),
                a(
                    TestsRun,
                    "tests.run",
                    t::ACTION_TESTS_RUN,
                    Some(Menu::Tests),
                    sc(CTRL_SHIFT, Key::F10),
                    false,
                ),
                a(
                    TestsStop,
                    "tests.stop",
                    t::ACTION_TESTS_STOP,
                    Some(Menu::Tests),
                    sc(CTRL, Key::F2),
                    false,
                ),
                a(
                    TestsRerun,
                    "tests.rerun",
                    t::ACTION_TESTS_RERUN,
                    Some(Menu::Tests),
                    sc(CTRL, Key::F5),
                    false,
                ),
            ],
        }
    }

    pub fn all(&self) -> &[Action] {
        &self.actions
    }

    pub fn get(&self, id: ActionId) -> &Action {
        self.actions
            .iter()
            .find(|a| a.id == id)
            .expect("action enregistrée")
    }

    pub fn by_key(&self, key: &str) -> Option<ActionId> {
        self.actions.iter().find(|a| a.key == key).map(|a| a.id)
    }

    /// « Ctrl+Maj+N » : le raccourci tel qu'on l'écrit dans les menus.
    pub fn shortcut_text(&self, id: ActionId) -> Option<String> {
        self.get(id).shortcut.as_ref().map(format_shortcut)
    }

    /// Change le raccourci d'une action. Refus (et rien ne change) si un autre
    /// l'utilise déjà : on rend alors l'action en conflit.
    #[allow(dead_code)] // réglage des raccourcis : pas encore d'écran, testé
    pub fn set_shortcut(
        &mut self,
        id: ActionId,
        shortcut: Option<KeyboardShortcut>,
    ) -> Result<(), ActionId> {
        if let Some(new) = &shortcut {
            if let Some(other) = self
                .actions
                .iter()
                .find(|a| a.id != id && a.shortcut.as_ref() == Some(new))
            {
                return Err(other.id);
            }
        }
        if let Some(action) = self.actions.iter_mut().find(|a| a.id == id) {
            action.shortcut = shortcut;
        }
        Ok(())
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// `Ctrl+Maj+N`, `Alt+Gauche`, `Alt+F12`… (textes français).
pub fn format_shortcut(shortcut: &KeyboardShortcut) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if shortcut.modifiers.ctrl {
        parts.push("Ctrl");
    }
    if shortcut.modifiers.alt {
        parts.push("Alt");
    }
    if shortcut.modifiers.shift {
        parts.push("Maj");
    }
    let key = match shortcut.logical_key {
        Key::ArrowLeft => "Gauche".to_owned(),
        Key::ArrowRight => "Droite".to_owned(),
        Key::ArrowUp => "Haut".to_owned(),
        Key::ArrowDown => "Bas".to_owned(),
        Key::Escape => "Échap".to_owned(),
        Key::Enter => "Entrée".to_owned(),
        other => other.name().to_owned(),
    };
    let mut text = parts.join("+");
    if !text.is_empty() {
        text.push('+');
    }
    text.push_str(&key);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_actions_share_a_shortcut() {
        let registry = Registry::new();
        for a in registry.all() {
            for b in registry.all() {
                if a.id != b.id && a.shortcut.is_some() {
                    assert_ne!(a.shortcut, b.shortcut, "{} et {}", a.key, b.key);
                }
            }
        }
    }

    #[test]
    fn shortcut_conflicts_are_refused() {
        let mut registry = Registry::new();
        let taken = registry.get(ActionId::SaveAll).shortcut;
        assert_eq!(
            registry.set_shortcut(ActionId::Quit, taken),
            Err(ActionId::SaveAll)
        );
        assert!(registry.get(ActionId::Quit).shortcut.is_none());
    }

    #[test]
    fn shortcuts_are_written_in_french() {
        let registry = Registry::new();
        assert_eq!(
            registry.shortcut_text(ActionId::GotoFile).as_deref(),
            Some("Ctrl+Maj+N")
        );
        assert_eq!(
            registry.shortcut_text(ActionId::PreviousTab).as_deref(),
            Some("Alt+Gauche")
        );
    }

    #[test]
    fn keys_are_unique_and_found() {
        let registry = Registry::new();
        assert_eq!(registry.by_key("file.save_all"), Some(ActionId::SaveAll));
        assert_eq!(registry.by_key("nope"), None);
    }
}
