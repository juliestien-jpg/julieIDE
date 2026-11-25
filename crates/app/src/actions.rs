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
    // ----- Éditeurs fractionnés (split view) -----------------------------------------
    SplitRight,
    SplitDown,
    MoveRight,
    MoveDown,
    MoveToOtherGroup,
    UnsplitAll,
    NextGroup,
    PrevGroup,
    GitCommit,
    GitRefresh,
    TestsRun,
    TestsStop,
    TestsRerun,
    Settings,
    // ----- v1 : Git global ----------------------------------------------------
    UpdateProject,
    Push,
    NewBranch,
    BranchesPopup,
    Fetch,
    ContextMenuKey,
    // ----- v1 : fichiers (arbre, changements, onglets) -------------------------
    GitAdd,
    GitUnstage,
    GitDiscard,
    GitShowDiff,
    GitFileHistory,
    IgnoreFile,
    IgnoreFolder,
    IgnoreExtension,
    OpenEntry,
    CopyPath,
    CopyRelativePath,
    CopyName,
    RevealInExplorer,
    OpenTerminalHere,
    FindInFolder,
    TabCloseOthers,
    TabCloseAll,
    TabCloseUnmodified,
    // ----- v1 bis : éditeur, arbre, tests, recherche, terminal ---------------------
    EditCut,
    EditCopy,
    EditPaste,
    EditSelectAll,
    GotoDeclaration,
    FindUsages,
    NavBack,
    NavForward,
    UsageNext,
    UsagePrevious,
    ToggleUsages,
    RunTestsHere,
    TreeNewFile,
    TreeNewFolder,
    TreeRename,
    TreeDelete,
    TestRunNode,
    TestRerunFailed,
    TestGoto,
    TestCopyName,
    TestCompare,
    HitOpen,
    HitCopyLine,
    HitExclude,
    TermCopy,
    TermPaste,
    TermSelectAll,
    TermClear,
    TermRename,
    TermClose,
    TermNewCmd,
    TermNewGitBash,
    // ----- v1 : journal ---------------------------------------------------------
    LogNewBranch,
    LogCheckout,
    LogCherryPick,
    LogRevert,
    LogReset,
    CopyHash,
    CopyShortHash,
    CopyMessage,
    // ----- v1 : branche (fenêtre surgissante, arbre du journal) -------------------
    BranchCheckout,
    BranchNewFrom,
    BranchCheckoutRebase,
    BranchCompare,
    BranchRebaseOnto,
    BranchMerge,
    BranchUpdate,
    BranchPullMerge,
    BranchPullRebase,
    BranchPush,
    BranchRename,
    BranchDelete,
    BranchSetUpstream,
    BranchUnsetUpstream,
    BranchDiffWorktree,
    /// Remplacement de Ctrl+Maj+` pour les claviers où l'accent grave est peu accessible.
    BranchesPopupAlt,
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
    /// Raccourci **affiché** seulement : la touche est traitée par la zone elle-même
    /// (éditeur, arbre, terminal), pas par la distribution globale des raccourcis.
    pub shown: Option<&'static str>,
}

impl Action {
    fn shown(mut self, text: &'static str) -> Self {
        self.shown = Some(text);
        self
    }
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
            shown: None,
        };
        use ActionId::*;
        let mut actions = vec![
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
                Settings,
                "file.settings",
                t::ACTION_SETTINGS,
                Some(Menu::File),
                sc(CTRL_ALT, Key::S),
                true,
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
            // Fractionner : pas de raccourci par défaut (docs/specs/split.md) ; passer d'un
            // groupe à l'autre : Ctrl+Alt+Page suivante / précédente (Alt+Tab et
            // Ctrl+Alt+Tab appartiennent à Windows).
            a(
                SplitRight,
                "editor.split_right",
                t::ACTION_SPLIT_RIGHT,
                Some(Menu::View),
                None,
                false,
            ),
            a(
                SplitDown,
                "editor.split_down",
                t::ACTION_SPLIT_DOWN,
                Some(Menu::View),
                None,
                false,
            ),
            a(
                MoveRight,
                "editor.move_right",
                t::ACTION_MOVE_RIGHT,
                Some(Menu::View),
                None,
                false,
            ),
            a(
                MoveDown,
                "editor.move_down",
                t::ACTION_MOVE_DOWN,
                Some(Menu::View),
                None,
                false,
            ),
            a(
                MoveToOtherGroup,
                "editor.move_other",
                t::ACTION_MOVE_OTHER,
                Some(Menu::View),
                None,
                false,
            ),
            a(
                UnsplitAll,
                "editor.unsplit_all",
                t::ACTION_UNSPLIT_ALL,
                Some(Menu::View),
                None,
                false,
            ),
            a(
                NextGroup,
                "editor.next_group",
                t::ACTION_NEXT_GROUP,
                Some(Menu::View),
                sc(CTRL_ALT, Key::PageDown),
                false,
            ),
            a(
                PrevGroup,
                "editor.prev_group",
                t::ACTION_PREV_GROUP,
                Some(Menu::View),
                sc(CTRL_ALT, Key::PageUp),
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
        ];
        // ----- v1 : les actions Git, de fichiers, de journal et de branches --------
        // Un menu contextuel et le menu Git partagent ces définitions.
        let git = |id, key, label, shortcut| a(id, key, label, Some(Menu::Git), shortcut, false);
        let ctx = |id, key, label, shortcut| a(id, key, label, None, shortcut, false);
        actions.extend([
            git(
                UpdateProject,
                "vcs.update",
                t::ACTION_UPDATE_PROJECT,
                sc(CTRL, Key::T),
            ),
            git(Push, "vcs.push", t::ACTION_PUSH, sc(CTRL_SHIFT, Key::K)),
            git(NewBranch, "vcs.new_branch", t::ACTION_NEW_BRANCH, None),
            // Ctrl+Maj+` : réflexe PhpStorm (peu commode en AZERTY) ; le remplacement
            // Ctrl+Maj+B (`BranchesPopupAlt`) ouvre la même fenêtre.
            git(
                BranchesPopup,
                "vcs.branches",
                t::ACTION_BRANCHES,
                sc(CTRL_SHIFT, Key::Backtick),
            ),
            // Remplacement sans accent grave (m6 de la recette) : même fenêtre, Ctrl+Maj+B.
            ctx(
                BranchesPopupAlt,
                "vcs.branches_alt",
                t::ACTION_BRANCHES_ALT,
                sc(CTRL_SHIFT, Key::B),
            ),
            git(Fetch, "vcs.fetch", t::ACTION_FETCH, None),
            // La touche Menu reste à l'IDE, même dans le terminal.
            a(
                ContextMenuKey,
                "ui.context_menu",
                t::ACTION_CONTEXT_MENU,
                None,
                sc(Modifiers::NONE, Key::Menu),
                true,
            ),
            ctx(GitAdd, "vcs.add", t::ACTION_GIT_ADD, sc(CTRL_ALT, Key::A)),
            ctx(GitUnstage, "vcs.unstage", t::ACTION_GIT_UNSTAGE, None),
            ctx(
                GitDiscard,
                "vcs.discard",
                t::ACTION_GIT_DISCARD,
                sc(CTRL_ALT, Key::Z),
            ),
            ctx(GitShowDiff, "vcs.show_diff", t::ACTION_GIT_SHOW_DIFF, None),
            ctx(
                GitFileHistory,
                "vcs.file_history",
                t::ACTION_GIT_FILE_HISTORY,
                None,
            ),
            ctx(IgnoreFile, "vcs.ignore_file", t::ACTION_IGNORE_FILE, None),
            ctx(
                IgnoreFolder,
                "vcs.ignore_folder",
                t::ACTION_IGNORE_FOLDER,
                None,
            ),
            ctx(
                IgnoreExtension,
                "vcs.ignore_extension",
                t::ACTION_IGNORE_EXTENSION,
                None,
            ),
            ctx(OpenEntry, "file.open", t::ACTION_OPEN_ENTRY, None),
            ctx(
                CopyPath,
                "file.copy_path",
                t::ACTION_COPY_PATH,
                sc(CTRL_SHIFT, Key::C),
            ),
            ctx(
                CopyRelativePath,
                "file.copy_relative_path",
                t::ACTION_COPY_RELATIVE_PATH,
                None,
            ),
            ctx(CopyName, "file.copy_name", t::ACTION_COPY_NAME, None),
            ctx(RevealInExplorer, "file.reveal", t::ACTION_REVEAL, None),
            ctx(
                OpenTerminalHere,
                "terminal.open_here",
                t::ACTION_TERMINAL_HERE,
                None,
            ),
            ctx(
                FindInFolder,
                "nav.find_in_folder",
                t::ACTION_FIND_IN_FOLDER,
                None,
            ),
            ctx(
                TabCloseOthers,
                "tab.close_others",
                t::ACTION_TAB_CLOSE_OTHERS,
                None,
            ),
            ctx(TabCloseAll, "tab.close_all", t::ACTION_TAB_CLOSE_ALL, None),
            ctx(
                TabCloseUnmodified,
                "tab.close_unmodified",
                t::ACTION_TAB_CLOSE_UNMODIFIED,
                None,
            ),
            ctx(
                LogNewBranch,
                "log.new_branch",
                t::ACTION_LOG_NEW_BRANCH,
                None,
            ),
            ctx(LogCheckout, "log.checkout", t::ACTION_LOG_CHECKOUT, None),
            ctx(
                LogCherryPick,
                "log.cherry_pick",
                t::ACTION_LOG_CHERRY_PICK,
                None,
            ),
            ctx(LogRevert, "log.revert", t::ACTION_LOG_REVERT, None),
            ctx(LogReset, "log.reset", t::ACTION_LOG_RESET, None),
            ctx(CopyHash, "log.copy_hash", t::ACTION_COPY_HASH, None),
            ctx(
                CopyShortHash,
                "log.copy_short_hash",
                t::ACTION_COPY_SHORT_HASH,
                None,
            ),
            ctx(
                CopyMessage,
                "log.copy_message",
                t::ACTION_COPY_MESSAGE,
                None,
            ),
            ctx(
                BranchCheckout,
                "branch.checkout",
                t::ACTION_BRANCH_CHECKOUT,
                None,
            ),
            ctx(
                BranchNewFrom,
                "branch.new_from",
                t::ACTION_BRANCH_NEW_FROM,
                None,
            ),
            ctx(
                BranchCheckoutRebase,
                "branch.checkout_rebase",
                t::ACTION_BRANCH_CHECKOUT_REBASE,
                None,
            ),
            ctx(
                BranchCompare,
                "branch.compare",
                t::ACTION_BRANCH_COMPARE,
                None,
            ),
            ctx(
                BranchRebaseOnto,
                "branch.rebase_onto",
                t::ACTION_BRANCH_REBASE_ONTO,
                None,
            ),
            ctx(BranchMerge, "branch.merge", t::ACTION_BRANCH_MERGE, None),
            ctx(BranchUpdate, "branch.update", t::ACTION_BRANCH_UPDATE, None),
            ctx(
                BranchPullMerge,
                "branch.pull_merge",
                t::ACTION_BRANCH_PULL_MERGE,
                None,
            ),
            ctx(
                BranchPullRebase,
                "branch.pull_rebase",
                t::ACTION_BRANCH_PULL_REBASE,
                None,
            ),
            ctx(BranchPush, "branch.push", t::ACTION_BRANCH_PUSH, None),
            ctx(BranchRename, "branch.rename", t::ACTION_BRANCH_RENAME, None),
            ctx(BranchDelete, "branch.delete", t::ACTION_BRANCH_DELETE, None),
            ctx(
                BranchSetUpstream,
                "branch.set_upstream",
                t::ACTION_BRANCH_SET_UPSTREAM,
                None,
            ),
            ctx(
                BranchUnsetUpstream,
                "branch.unset_upstream",
                t::ACTION_BRANCH_UNSET_UPSTREAM,
                None,
            ),
            ctx(
                BranchDiffWorktree,
                "branch.diff_worktree",
                t::ACTION_BRANCH_DIFF_WORKTREE,
                None,
            ),
        ]);
        // ----- v1 bis : menus de l'éditeur, de l'arbre, des tests, de la recherche, du terminal.
        // Les raccourcis sont seulement affichés (voir `Action::shown`) : la zone les traite.
        actions.extend([
            ctx(EditCut, "edit.cut", t::ACTION_EDIT_CUT, None).shown("Ctrl+X"),
            ctx(EditCopy, "edit.copy", t::ACTION_EDIT_COPY, None).shown("Ctrl+C"),
            ctx(EditPaste, "edit.paste", t::ACTION_EDIT_PASTE, None).shown("Ctrl+V"),
            ctx(
                EditSelectAll,
                "edit.select_all",
                t::ACTION_EDIT_SELECT_ALL,
                None,
            )
            .shown("Ctrl+A"),
            // Ctrl+B est traité par l'éditeur (il a besoin de l'ancre sous le mot) ; les autres
            // touches de la navigation sont des raccourcis globaux (pas dans le terminal).
            a(
                GotoDeclaration,
                "nav.declaration",
                t::ACTION_GOTO_DECLARATION,
                Some(Menu::Navigate),
                None,
                false,
            )
            .shown("Ctrl+B"),
            a(
                FindUsages,
                "nav.usages",
                t::ACTION_FIND_USAGES,
                Some(Menu::Navigate),
                sc(ALT, Key::F7),
                false,
            ),
            a(
                NavBack,
                "nav.back",
                t::ACTION_NAV_BACK,
                Some(Menu::Navigate),
                sc(CTRL_ALT, Key::ArrowLeft),
                false,
            ),
            a(
                NavForward,
                "nav.forward",
                t::ACTION_NAV_FORWARD,
                Some(Menu::Navigate),
                sc(CTRL_ALT, Key::ArrowRight),
                false,
            ),
            a(
                UsageNext,
                "nav.usage_next",
                t::ACTION_USAGE_NEXT,
                None,
                sc(CTRL_ALT, Key::ArrowDown),
                false,
            ),
            a(
                UsagePrevious,
                "nav.usage_previous",
                t::ACTION_USAGE_PREVIOUS,
                None,
                sc(CTRL_ALT, Key::ArrowUp),
                false,
            ),
            a(
                ToggleUsages,
                "view.usages",
                t::ACTION_TOGGLE_USAGES,
                Some(Menu::View),
                None,
                false,
            ),
            ctx(
                RunTestsHere,
                "tests.run_here",
                t::ACTION_RUN_TESTS_HERE,
                None,
            ),
            ctx(TreeNewFile, "tree.new_file", t::ACTION_TREE_NEW_FILE, None).shown("Alt+Inser"),
            ctx(
                TreeNewFolder,
                "tree.new_folder",
                t::ACTION_TREE_NEW_FOLDER,
                None,
            ),
            ctx(TreeRename, "tree.rename", t::ACTION_TREE_RENAME, None).shown("Maj+F6"),
            ctx(TreeDelete, "tree.delete", t::ACTION_TREE_DELETE, None).shown("Suppr"),
            ctx(TestRunNode, "tests.run_node", t::ACTION_TEST_RUN_NODE, None),
            ctx(
                TestRerunFailed,
                "tests.rerun_failed",
                t::ACTION_TEST_RERUN_FAILED,
                None,
            ),
            ctx(TestGoto, "tests.goto", t::ACTION_TEST_GOTO, None).shown("F4"),
            ctx(
                TestCopyName,
                "tests.copy_name",
                t::ACTION_TEST_COPY_NAME,
                None,
            )
            .shown("Ctrl+C"),
            ctx(TestCompare, "tests.compare", t::ACTION_TEST_COMPARE, None),
            ctx(HitOpen, "search.open", t::ACTION_HIT_OPEN, None).shown("Entrée"),
            ctx(
                HitCopyLine,
                "search.copy_line",
                t::ACTION_HIT_COPY_LINE,
                None,
            )
            .shown("Ctrl+C"),
            ctx(HitExclude, "search.exclude", t::ACTION_HIT_EXCLUDE, None),
            ctx(TermCopy, "terminal.copy", t::ACTION_TERM_COPY, None).shown("Ctrl+Maj+C"),
            ctx(TermPaste, "terminal.paste", t::ACTION_TERM_PASTE, None).shown("Ctrl+Maj+V"),
            ctx(
                TermSelectAll,
                "terminal.select_all",
                t::ACTION_TERM_SELECT_ALL,
                None,
            ),
            ctx(TermClear, "terminal.clear", t::ACTION_TERM_CLEAR, None),
            ctx(TermRename, "terminal.rename", t::ACTION_TERM_RENAME, None),
            ctx(TermClose, "terminal.close", t::ACTION_TERM_CLOSE, None).shown("Ctrl+F4"),
            ctx(TermNewCmd, "terminal.new_cmd", t::ACTION_TERM_NEW_CMD, None),
            ctx(
                TermNewGitBash,
                "terminal.new_git_bash",
                t::ACTION_TERM_NEW_BASH,
                None,
            ),
        ]);
        Self { actions }
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
        let action = self.get(id);
        action
            .shown
            .map(str::to_owned)
            .or_else(|| action.shortcut.as_ref().map(format_shortcut))
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
