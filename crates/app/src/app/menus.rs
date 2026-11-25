//! Menus contextuels côté application : ce sur quoi ils agissent (`Target`), les
//! entrées de chaque zone (tirées du registre d'actions : libellé et raccourci
//! d'une seule définition), leur disponibilité avec la raison du grisé, et
//! l'exécution de l'action choisie.
//!
//! Aucune règle Git ici : les raisons viennent de l'état publié par `julie-vcs`
//! (`Status`, `BranchList`) ; les écritures partent vers `Vcs`.

use super::*;

use julie_vcs::{
    Change, FetchRequest, FileKind, FileStatus, Group, IgnoreRule, LocalChanges, MergeRequest,
    OperationKind, PullMode, PullRequest, RebaseRequest, SwitchTarget,
};

use crate::branchui::BranchRef;
use crate::ctxmenu::{ContextMenu, Entry, Item, Outcome as MenuOutcome};
use crate::gitdialogs as gd;
use crate::target::{CommitRef, EditorRef, FileRef, HitRef, Target, TermRef, TestRef, Zone};
use crate::texts as t;
use julie_phpunit::{NodeKind, RunRequest};

/// Les fichiers de l'état Git concernés par ces cibles (dossier : ce qu'il contient).
fn scope<'a>(status: &'a julie_vcs::Status, files: &[FileRef]) -> Vec<&'a FileStatus> {
    status
        .files
        .iter()
        .filter(|f| {
            files.iter().any(|t| {
                if t.is_dir {
                    f.path.starts_with(&t.path)
                } else {
                    f.path == t.path || f.original_path.as_deref() == Some(t.path.as_path())
                }
            })
        })
        .collect()
}

fn can_add(f: &FileStatus) -> bool {
    f.group() != Group::Conflicted
        && (f.kind == FileKind::Untracked || f.unstaged != Change::Unmodified)
}

fn can_unstage(f: &FileStatus) -> bool {
    f.group() != Group::Conflicted && f.staged != Change::Unmodified
}

fn can_discard(f: &FileStatus) -> bool {
    f.group() == Group::Changed
}

/// Ce que « Lancer les tests » ferait d'un chemin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RunKind {
    File,
    Dir,
    Config,
}

/// Fichier `*Test.php`, dossier, ou configuration PHPUnit ; rien d'autre.
pub(super) fn tests_path_kind(file: &FileRef) -> Option<RunKind> {
    if file.is_dir {
        return Some(RunKind::Dir);
    }
    let name = file.path.file_name()?.to_string_lossy().to_lowercase();
    match name.as_str() {
        "phpunit.xml" | "phpunit.xml.dist" | "phpunit.dist.xml" => Some(RunKind::Config),
        n if n.ends_with("test.php") => Some(RunKind::File),
        _ => None,
    }
}

/// Le fichier visé par une cible de l'éditeur ou de la recherche.
fn target_file(target: &Target) -> Option<FileRef> {
    match target {
        Target::Files { files, .. } => files.first().cloned(),
        Target::Editor(EditorRef {
            path: Some(path), ..
        }) => Some(FileRef {
            path: path.clone(),
            is_dir: false,
        }),
        Target::Hit(hit) => Some(FileRef {
            path: hit.path.clone(),
            is_dir: false,
        }),
        _ => None,
    }
}

/// Une cible de l'éditeur ou de la recherche vue comme un fichier de l'onglet.
fn file_view(target: &Target) -> Option<Target> {
    if matches!(target, Target::Editor(_) | Target::Hit(_)) {
        target_file(target).map(|file| Target::Files {
            zone: Zone::Tab,
            files: vec![file],
        })
    } else {
        None
    }
}

/// L'action est-elle propre à la zone de cette cible (et non une action sur un fichier) ?
fn native_to(id: ActionId, target: &Target) -> bool {
    use ActionId::*;
    match target {
        Target::Editor(_) => matches!(
            id,
            EditCut | EditCopy | EditPaste | EditSelectAll | GotoDeclaration | FindUsages
        ),
        Target::Hit(_) => matches!(id, HitOpen | HitCopyLine | HitExclude),
        _ => true,
    }
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl JulieStormApp {
    // ----- Cible courante --------------------------------------------------------

    /// Ce sur quoi agit une action appelée sans menu (raccourci, palette, scénario) :
    /// la zone qui a le focus, à défaut la zone visible.
    pub(super) fn current_target(&self) -> Option<Target> {
        if let Some(popup) = &self.branch_popup {
            return popup.selected_branch().map(Target::Branch);
        }
        let ctx = &self.ctx;
        let tree = || {
            let files = self.tree.selection_refs();
            (!files.is_empty()).then_some(Target::Files {
                zone: Zone::Tree,
                files,
            })
        };
        let changes = || {
            let status = self.git.status()?;
            let files: Vec<FileRef> = self
                .git
                .selection_paths()
                .into_iter()
                .filter(|p| status.find(p).is_some())
                .map(|path| FileRef {
                    path,
                    is_dir: false,
                })
                .collect();
            (!files.is_empty()).then_some(Target::Files {
                zone: Zone::Changes,
                files,
            })
        };
        let log = || self.git.log.selected_commit().map(Target::Commit);
        let tab = || {
            self.editors
                .active_editor()
                .and_then(|e| e.path().map(Path::to_path_buf))
                .map(|path| Target::Files {
                    zone: Zone::Tab,
                    files: vec![FileRef {
                        path,
                        is_dir: false,
                    }],
                })
        };
        if self.tree.has_focus(ctx) {
            return tree();
        }
        let git_visible = self.tools.bottom_open && self.tools.bottom == BottomTool::Git;
        if git_visible && self.git.log.has_focus(ctx) {
            return log();
        }
        if git_visible && self.git.list_has_focus(ctx) {
            return changes();
        }
        if self
            .editors
            .active_editor()
            .is_some_and(|e| e.view.has_focus(ctx))
        {
            return tab();
        }
        if git_visible {
            let found = match self.git.tab {
                GitTab::Log => log(),
                GitTab::Changes => changes(),
                GitTab::Console => None,
            };
            if found.is_some() {
                return found;
            }
        }
        tree().or_else(tab)
    }

    // ----- Ouverture -------------------------------------------------------------

    pub(super) fn open_menu(&mut self, target: Target, pos: egui::Pos2, by_keyboard: bool) {
        let items = self.menu_items(&target);
        let entries = self.resolve_items(&items, &target);
        if entries.is_empty() {
            return;
        }
        let zone = match &target {
            Target::Files { zone, .. } => zone.key(),
            Target::Commit(_) => "log",
            Target::Branch(_) => "branch",
            Target::Test(_) => "test",
            Target::Hit(_) => "search",
            Target::Editor(_) => "editor",
            Target::Terminal(_) => "terminal",
        };
        let description = target.describe();
        self.ctx_target = Some(target);
        self.ctx_menu = Some(ContextMenu::new(
            zone,
            description,
            pos,
            entries,
            by_keyboard,
        ));
    }

    /// Touche Menu : le menu de la zone qui a le focus, sous l'élément choisi.
    pub(super) fn open_menu_for_focus(&mut self) {
        let ctx = self.ctx.clone();
        let screen = ctx.content_rect();
        // Zones qui ne passent pas par `current_target` : terminal, tests, éditeur.
        if self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.has_focus(&ctx))
        {
            if let Some(target) = self.terminal_target(false) {
                let pos = egui::pos2(screen.left() + 200.0, screen.bottom() - 200.0);
                return self.open_menu(target, pos, true);
            }
        }
        let tests_visible = self.tools.bottom_open && self.tools.bottom == BottomTool::Tests;
        if tests_visible && self.tests.has_focus(&ctx) {
            if let Some(target) = self.tests.selected_ref().map(Target::Test) {
                let pos = egui::pos2(screen.left() + 200.0, screen.bottom() - 180.0);
                return self.open_menu(target, pos, true);
            }
        }
        if self
            .editors
            .active_editor()
            .is_some_and(|e| e.view.has_focus(&ctx))
        {
            let pos = egui::pos2(screen.center().x, screen.top() + 200.0);
            let target = self.editor_target();
            return self.open_menu(target, pos, true);
        }
        let Some(target) = self.current_target() else {
            return;
        };
        let pos = match &target {
            Target::Branch(_) => egui::pos2(screen.right() - 440.0, 140.0),
            Target::Files {
                zone: Zone::Tree, ..
            } => egui::pos2(120.0, 160.0),
            Target::Files {
                zone: Zone::Tab, ..
            } => egui::pos2(260.0, 60.0),
            _ => egui::pos2(screen.left() + 160.0, screen.bottom() - 260.0),
        };
        self.open_menu(target, pos, true);
    }

    /// Ce que le menu de l'éditeur voit : le fichier, la sélection, la lecture seule.
    pub(super) fn editor_target(&self) -> Target {
        let editor = self.editors.active_editor();
        Target::Editor(EditorRef {
            path: editor.and_then(|e| e.path().map(Path::to_path_buf)),
            has_selection: editor.is_some_and(|e| !e.document.selections().primary().is_empty()),
            read_only: editor.is_some_and(|e| e.document.is_read_only()),
        })
    }

    pub(super) fn terminal_target(&self, on_tab: bool) -> Option<Target> {
        let terminal = self.terminal.as_ref()?;
        Some(Target::Terminal(TermRef {
            has_selection: terminal.has_selection(),
            tab: terminal.active_index(),
            on_tab,
        }))
    }

    pub(super) fn collect_menu_requests(&mut self) {
        // Suppr, Maj+F6, Alt+Inser dans l'arbre.
        if let Some(id) = self.tree.key_action.take() {
            let target = self.current_target();
            if let Some(reason) = target.as_ref().and_then(|t| self.availability(id, t)) {
                self.toast(reason, true, None);
            } else {
                self.run_v1_action(id, target);
            }
        }
        let request = self
            .tree
            .menu_request
            .take()
            .or_else(|| self.editors.menu_request.take())
            .or_else(|| self.git.menu_request.take())
            .or_else(|| self.tests.menu_request.take())
            .or_else(|| self.find.menu_request.take())
            .or_else(|| {
                self.terminal
                    .as_mut()
                    .and_then(|terminal| terminal.menu_request.take())
            });
        if let Some(request) = request {
            // Le menu d'une occurrence s'ouvre sur la fenêtre de recherche, qui reste là.
            if !matches!(request.target, Target::Hit(_)) {
                self.close_floating_keep_popup();
            }
            self.open_menu(request.target, request.pos, request.by_keyboard);
        }
        // Clic droit dans l'éditeur : le curseur se place d'abord au point cliqué (par un
        // vrai clic gauche rejoué), puis le menu s'ouvre quelques images plus tard.
        if let Some(click) = self.editors.editor_click.take() {
            self.close_floating_keep_popup();
            if !click.inside_selection {
                self.injected
                    .extend(crate::scenario::Injection::click(click.pos));
            }
            self.deferred_click_pos = click.pos;
            self.deferred_menu = Some((
                if click.inside_selection { 1 } else { 6 },
                Target::Editor(EditorRef {
                    path: None,
                    has_selection: false,
                    read_only: false,
                }),
            ));
        }
        if let Some((frames, _)) = &mut self.deferred_menu {
            if *frames > 0 {
                *frames -= 1;
                self.ctx.request_repaint();
            } else {
                self.deferred_menu = None;
                let target = self.editor_target();
                let pos = self.deferred_click_pos;
                self.open_menu(target, pos, false);
            }
        }
    }

    fn close_floating_keep_popup(&mut self) {
        self.goto = None;
        self.find_open = false;
        self.palette = None;
    }

    pub(super) fn draw_ctx_menu(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(menu) = &mut self.ctx_menu else {
            return;
        };
        match menu.show(ctx, p) {
            MenuOutcome::Open => {}
            MenuOutcome::Close => {
                self.ctx_menu = None;
                self.ctx_target = None;
            }
            MenuOutcome::Run(id) => self.finish_menu(id),
        }
    }

    /// Une entrée est choisie (clic, Entrée, scénario).
    pub(super) fn finish_menu(&mut self, id: ActionId) {
        let target = self.ctx_target.take();
        self.ctx_menu = None;
        if let Some(target) = target {
            if matches!(target, Target::Branch(_)) {
                self.branch_popup = None;
            }
            if matches!(id, ActionId::FindInFiles) {
                // Action globale ; la recherche part de la sélection de l'éditeur.
                self.run_action(id);
            } else {
                self.run_v1_action(id, Some(target));
            }
        }
        self.ctx.request_repaint();
    }

    // ----- Contenu des menus -----------------------------------------------------

    fn menu_items(&self, target: &Target) -> Vec<Item> {
        use ActionId::*;
        use Item::{Action, Sep, Sub};
        let copy = || {
            Sub(
                "sub.copy",
                t::MENU_COPY_PATH.to_owned(),
                vec![Action(CopyPath), Action(CopyRelativePath), Action(CopyName)],
            )
        };
        match target {
            Target::Files {
                zone: Zone::Tree, ..
            } => vec![
                Sub(
                    "sub.new",
                    t::MENU_NEW.to_owned(),
                    vec![Action(TreeNewFile), Action(TreeNewFolder)],
                ),
                Action(OpenEntry),
                Sep,
                Sub(
                    "sub.git",
                    t::MENU_GIT.to_owned(),
                    vec![
                        Action(GitAdd),
                        Action(GitUnstage),
                        Action(GitDiscard),
                        Sep,
                        Action(GitShowDiff),
                        Action(GitFileHistory),
                        Sep,
                        Action(IgnoreFile),
                        Action(IgnoreFolder),
                        Action(IgnoreExtension),
                    ],
                ),
                Sep,
                Action(RunTestsHere),
                Sep,
                copy(),
                Action(TreeRename),
                Action(TreeDelete),
                Sep,
                Action(FindInFolder),
                Action(RevealInExplorer),
                Action(OpenTerminalHere),
            ],
            Target::Files {
                zone: Zone::Changes,
                ..
            } => vec![
                Action(GitShowDiff),
                Action(OpenEntry),
                Sep,
                Action(GitAdd),
                Action(GitUnstage),
                Action(GitDiscard),
                Sep,
                Action(GitFileHistory),
                Sub(
                    "sub.ignore",
                    t::MENU_IGNORE.to_owned(),
                    vec![
                        Action(IgnoreFile),
                        Action(IgnoreFolder),
                        Action(IgnoreExtension),
                    ],
                ),
                Sep,
                Action(TreeDelete),
                Sep,
                copy(),
                Action(RevealInExplorer),
            ],
            Target::Files {
                zone: Zone::Tab, ..
            } => vec![
                Action(CloseTab),
                Action(TabCloseOthers),
                Action(TabCloseAll),
                Action(TabCloseUnmodified),
                Sep,
                copy(),
                Action(RevealInExplorer),
                Sep,
                Sub(
                    "sub.git",
                    t::MENU_GIT.to_owned(),
                    vec![
                        Action(GitShowDiff),
                        Action(GitDiscard),
                        Action(GitFileHistory),
                    ],
                ),
            ],
            Target::Editor(_) => vec![
                Action(EditCut),
                Action(EditCopy),
                Action(EditPaste),
                Sep,
                Action(EditSelectAll),
                Sep,
                Action(GotoDeclaration),
                Action(FindUsages),
                Sep,
                Action(FindInFiles),
                Action(RunTestsHere),
                Sep,
                copy(),
                Sep,
                Sub(
                    "sub.git",
                    t::MENU_GIT.to_owned(),
                    vec![
                        Action(GitShowDiff),
                        Action(GitDiscard),
                        Action(GitFileHistory),
                    ],
                ),
            ],
            Target::Test(_) => vec![
                Action(TestRunNode),
                Action(TestRerunFailed),
                Sep,
                Action(TestGoto),
                Action(TestCompare),
                Sep,
                Action(TestCopyName),
            ],
            Target::Hit(_) => vec![
                Action(HitOpen),
                Action(HitCopyLine),
                Sub(
                    "sub.copy",
                    t::MENU_COPY_PATH.to_owned(),
                    vec![Action(CopyPath), Action(CopyRelativePath), Action(CopyName)],
                ),
                Sep,
                Action(HitExclude),
                Action(FindInFolder),
                Sep,
                Action(RevealInExplorer),
            ],
            Target::Terminal(term) if term.on_tab => vec![
                Action(TermRename),
                Action(TermClose),
                Sep,
                Sub(
                    "sub.term_new",
                    t::MENU_TERM_NEW.to_owned(),
                    vec![Action(TermNewCmd), Action(TermNewGitBash)],
                ),
            ],
            Target::Terminal(_) => vec![
                Action(TermCopy),
                Action(TermPaste),
                Action(TermSelectAll),
                Action(TermClear),
                Sep,
                Sub(
                    "sub.term_new",
                    t::MENU_TERM_NEW.to_owned(),
                    vec![Action(TermNewCmd), Action(TermNewGitBash)],
                ),
                Action(TermRename),
                Action(TermClose),
            ],
            Target::Commit(_) => vec![
                Action(LogNewBranch),
                Action(LogCheckout),
                Sep,
                Action(LogCherryPick),
                Action(LogRevert),
                Action(LogReset),
                Sep,
                Action(CopyHash),
                Action(CopyShortHash),
                Action(CopyMessage),
            ],
            Target::Branch(_) => vec![
                Action(BranchCheckout),
                Action(BranchNewFrom),
                Action(BranchCheckoutRebase),
                Sep,
                Action(BranchCompare),
                Action(BranchRebaseOnto),
                Action(BranchMerge),
                Sep,
                Action(BranchUpdate),
                Action(BranchPullMerge),
                Action(BranchPullRebase),
                Action(BranchPush),
                Sep,
                Action(BranchRename),
                Action(BranchDelete),
            ],
        }
    }

    fn resolve_items(&self, items: &[Item], target: &Target) -> Vec<Entry> {
        let mut entries: Vec<Entry> = Vec::new();
        for item in items {
            match item {
                Item::Sep => {
                    if entries.last().is_some_and(|e| !e.separator) {
                        entries.push(Entry::separator());
                    }
                }
                Item::Action(id) => {
                    if !self.applicable(*id, target) {
                        continue;
                    }
                    let action = self.registry.get(*id);
                    entries.push(Entry {
                        key: action.key.to_owned(),
                        id: Some(*id),
                        label: self.ctx_label(*id, target),
                        shortcut: self.registry.shortcut_text(*id),
                        disabled: self.availability(*id, target),
                        destructive: matches!(
                            id,
                            ActionId::GitDiscard | ActionId::BranchDelete | ActionId::LogReset
                        ),
                        children: Vec::new(),
                        separator: false,
                    });
                }
                Item::Sub(key, label, children) => {
                    let mut resolved = self.resolve_items(children, target);
                    while resolved.last().is_some_and(|e| e.separator) {
                        resolved.pop();
                    }
                    if resolved.is_empty() {
                        continue;
                    }
                    let disabled = resolved
                        .iter()
                        .filter(|e| !e.separator)
                        .all(|e| e.disabled.is_some())
                        .then(|| {
                            resolved
                                .iter()
                                .find_map(|e| e.disabled.clone())
                                .unwrap_or_default()
                        });
                    entries.push(Entry {
                        key: (*key).to_owned(),
                        id: None,
                        label: label.clone(),
                        shortcut: None,
                        disabled,
                        destructive: false,
                        children: resolved,
                        separator: false,
                    });
                }
            }
        }
        while entries.last().is_some_and(|e| e.separator) {
            entries.pop();
        }
        entries
    }

    /// L'entrée a-t-elle un sens pour cette cible ? (Sinon elle n'apparaît pas.)
    fn applicable(&self, id: ActionId, target: &Target) -> bool {
        use ActionId::*;
        match target {
            Target::Files { files, .. } => match id {
                OpenEntry => files.iter().any(|f| !f.is_dir),
                IgnoreFile => files.iter().all(|f| !f.is_dir),
                IgnoreFolder => files.iter().any(|f| f.is_dir),
                IgnoreExtension => files
                    .iter()
                    .all(|f| !f.is_dir && f.path.extension().is_some()),
                _ => true,
            },
            Target::Commit(_) | Target::Test(_) | Target::Terminal(_) => true,
            Target::Editor(_) => true,
            Target::Hit(_) => true,
            Target::Branch(b) => {
                let current = self.is_current_branch(b);
                match id {
                    BranchCheckout | BranchCheckoutRebase | BranchCompare | BranchRebaseOnto
                    | BranchMerge => !current,
                    BranchUpdate | BranchPush | BranchRename => !b.remote,
                    BranchPullMerge | BranchPullRebase => b.remote,
                    _ => true,
                }
            }
        }
    }

    fn is_current_branch(&self, b: &BranchRef) -> bool {
        !b.remote && self.current_branch().as_deref() == Some(b.name.as_str())
    }

    /// Libellé de l'entrée : celui du registre, précisé quand la cible le demande.
    fn ctx_label(&self, id: ActionId, target: &Target) -> String {
        use ActionId::*;
        let registry_label = self.registry.get(id).label.to_owned();
        let current = self.current_branch().unwrap_or_default();
        match (id, target) {
            (BranchCheckoutRebase, Target::Branch(_)) => t::label_checkout_rebase(&current),
            (BranchCompare, Target::Branch(_)) => t::label_compare(&current),
            (BranchRebaseOnto, Target::Branch(b)) => t::label_rebase_onto(&current, &b.name),
            (BranchMerge, Target::Branch(b)) => t::label_merge_into(&b.name, &current),
            (BranchPullMerge, Target::Branch(_)) => t::label_pull_merge(&current),
            (BranchPullRebase, Target::Branch(_)) => t::label_pull_rebase(&current),
            (BranchNewFrom, Target::Branch(b)) => t::label_new_from(&b.name),
            (IgnoreExtension, Target::Files { files, .. }) => files
                .first()
                .and_then(|f| f.path.extension())
                .map_or(registry_label, |e| {
                    t::label_ignore_ext(&e.to_string_lossy())
                }),
            (
                OpenEntry,
                Target::Files {
                    zone: Zone::Changes,
                    ..
                },
            ) => t::LABEL_OPEN_FILE.to_owned(),
            (CloseTab, _) => t::LABEL_CLOSE_TAB.to_owned(),
            (TestRunNode, Target::Test(test)) => match test.kind {
                NodeKind::Root => t::LABEL_RUN_ALL,
                NodeKind::Suite => t::LABEL_RUN_SUITE,
                NodeKind::Class => t::LABEL_RUN_CLASS,
                NodeKind::Method | NodeKind::DataSet => t::LABEL_RUN_TEST,
            }
            .to_owned(),
            (RunTestsHere, target) => match target_file(target).map(|f| tests_path_kind(&f)) {
                Some(Some(RunKind::Dir)) => t::LABEL_RUN_TESTS_DIR,
                Some(Some(RunKind::Config)) => t::LABEL_RUN_TESTS_CONFIG,
                _ => t::LABEL_RUN_TESTS_FILE,
            }
            .to_owned(),
            _ => registry_label,
        }
    }

    /// `None` : l'entrée est active ; sinon la raison du grisé (infobulle).
    pub(super) fn availability(&self, id: ActionId, target: &Target) -> Option<String> {
        use ActionId::*;
        // Les entrées propres à l'éditeur, aux tests, à la recherche et au terminal.
        match (id, target) {
            (GotoDeclaration | FindUsages, _) => return Some(t::REASON_SOON.to_owned()),
            (FindInFiles, _) => {
                return self
                    .project
                    .is_none()
                    .then(|| t::REASON_NO_PROJECT_OPEN.to_owned())
            }
            (EditCut | EditPaste, Target::Editor(e)) if e.read_only => {
                return Some(t::REASON_READ_ONLY.to_owned())
            }
            (EditCut | EditCopy | EditPaste | EditSelectAll, _) => {
                return self
                    .editors
                    .active_editor()
                    .is_none()
                    .then(|| t::REASON_NOT_SAVED.to_owned())
            }
            (TestRunNode, Target::Test(test)) => {
                return if self.tests.service.is_none() {
                    Some(t::REASON_PHPUNIT_MISSING.to_owned())
                } else if self.tests.request_for_node(test.id).is_none() {
                    Some(t::REASON_NO_SOURCE.to_owned())
                } else {
                    None
                }
            }
            (TestRerunFailed, _) => {
                return if !self.tests.has_run() {
                    Some(t::REASON_NO_TESTS_RUN.to_owned())
                } else if self.tests.failures_request().is_none() {
                    Some(t::REASON_NO_FAILURE.to_owned())
                } else {
                    None
                }
            }
            (TestGoto, Target::Test(test)) => {
                return self
                    .tests
                    .location_of(test.id)
                    .is_none()
                    .then(|| t::REASON_NO_SOURCE.to_owned())
            }
            (TestCompare, Target::Test(test)) => {
                return (!self.tests.has_comparison(test.id))
                    .then(|| t::REASON_NO_COMPARISON.to_owned())
            }
            (TestCopyName | HitOpen | HitCopyLine | HitExclude, _) => return None,
            (TermCopy, Target::Terminal(term)) if !term.has_selection => {
                return Some(t::REASON_NO_TEXT_SELECTED.to_owned())
            }
            (TermNewCmd | TermNewGitBash, _) => {
                let kind = if id == TermNewCmd {
                    julie_terminal::ProfileKind::Cmd
                } else {
                    julie_terminal::ProfileKind::GitBash
                };
                return self
                    .terminal
                    .as_ref()
                    .and_then(|terminal| terminal.profile_problem(kind));
            }
            (TermCopy | TermPaste | TermSelectAll | TermClear | TermRename | TermClose, _) => {
                return self
                    .terminal
                    .as_ref()
                    .is_none_or(|terminal| !terminal.has_tabs())
                    .then(|| t::REASON_NO_TERMINAL.to_owned())
            }
            _ => {}
        }
        // Les autres entrées d'un menu de l'éditeur ou de la recherche portent sur le fichier.
        if let Some(files) = file_view(target) {
            return self.availability(id, &files);
        }
        let git_ids = matches!(
            id,
            GitAdd
                | GitUnstage
                | GitDiscard
                | GitShowDiff
                | GitFileHistory
                | IgnoreFile
                | IgnoreFolder
                | IgnoreExtension
                | LogNewBranch
                | LogCheckout
                | LogCherryPick
                | LogRevert
                | LogReset
                | BranchCheckout
                | BranchNewFrom
                | BranchCheckoutRebase
                | BranchCompare
                | BranchRebaseOnto
                | BranchMerge
                | BranchUpdate
                | BranchPullMerge
                | BranchPullRebase
                | BranchPush
                | BranchRename
                | BranchDelete
        );
        let status = self.git_status();
        if git_ids && status.is_none() {
            return Some(
                if matches!(
                    self.project.as_ref().map(|p| &p.git),
                    Some(GitInfo::NoRepository)
                ) {
                    t::REASON_NOT_IN_REPO
                } else {
                    t::REASON_GIT_LOADING
                }
                .to_owned(),
            );
        }
        let operation = status.is_some_and(|s| s.operation.is_some());
        let no_commit = status.is_some_and(|s| s.head.commit().is_none());
        match target {
            Target::Files { files, zone } => {
                let in_scope = status.map(|s| scope(s, files)).unwrap_or_default();
                match id {
                    GitAdd if !in_scope.iter().any(|f| can_add(f)) => {
                        Some(t::REASON_NOTHING_TO_ADD.to_owned())
                    }
                    GitUnstage if !in_scope.iter().any(|f| can_unstage(f)) => {
                        Some(t::REASON_NOTHING_STAGED.to_owned())
                    }
                    GitDiscard => {
                        if in_scope.iter().any(|f| f.group() == Group::Conflicted) {
                            Some(t::REASON_RESOLVE_CONFLICT.to_owned())
                        } else if !in_scope.iter().any(|f| can_discard(f)) {
                            Some(t::REASON_NOTHING_TO_DISCARD.to_owned())
                        } else {
                            None
                        }
                    }
                    GitShowDiff => {
                        if files.len() != 1 || files[0].is_dir {
                            Some(t::REASON_ONE_FILE.to_owned())
                        } else if in_scope.is_empty() {
                            Some(t::REASON_NO_CHANGE.to_owned())
                        } else if in_scope[0].group() == Group::Conflicted {
                            Some(t::REASON_RESOLVE_CONFLICT.to_owned())
                        } else {
                            None
                        }
                    }
                    GitFileHistory => {
                        if files.len() != 1 || files[0].is_dir {
                            Some(t::REASON_ONE_FILE.to_owned())
                        } else if in_scope.iter().any(|f| f.kind == FileKind::Untracked) {
                            Some(t::REASON_UNTRACKED.to_owned())
                        } else if no_commit {
                            Some(t::REASON_NO_COMMIT.to_owned())
                        } else {
                            None
                        }
                    }
                    IgnoreFile | IgnoreFolder | IgnoreExtension => {
                        if files.len() != 1 {
                            Some(t::REASON_ONE_FILE.to_owned())
                        } else if in_scope.iter().any(|f| f.kind != FileKind::Untracked) {
                            Some(t::REASON_ALREADY_TRACKED.to_owned())
                        } else {
                            None
                        }
                    }
                    TabCloseOthers if self.editors.items.len() <= 1 => {
                        Some(t::REASON_ONE_TAB.to_owned())
                    }
                    TabCloseUnmodified
                        if self.editors.items.iter().all(|e| e.document.is_modified()) =>
                    {
                        Some(t::REASON_NO_UNMODIFIED_TAB.to_owned())
                    }
                    TreeNewFile | TreeNewFolder if files.len() != 1 => {
                        Some(t::REASON_MULTI.to_owned())
                    }
                    TreeRename if files.len() != 1 => Some(t::REASON_MULTI.to_owned()),
                    TreeRename | TreeDelete
                        if *zone == Zone::Tree
                            && files
                                .iter()
                                .any(|f| Some(&f.path) == self.project_root().as_ref()) =>
                    {
                        Some(t::REASON_PROJECT_ROOT.to_owned())
                    }
                    TreeDelete
                        if *zone != Zone::Tree
                            && !in_scope.iter().any(|f| f.kind == FileKind::Untracked) =>
                    {
                        Some(t::REASON_NOTHING_TO_DELETE.to_owned())
                    }
                    RunTestsHere => {
                        if files.len() != 1 {
                            Some(t::REASON_ONE_FILE.to_owned())
                        } else if tests_path_kind(&files[0]).is_none() {
                            Some(t::REASON_NOT_A_TEST.to_owned())
                        } else if self.tests.service.is_none() {
                            Some(t::REASON_PHPUNIT_MISSING.to_owned())
                        } else {
                            None
                        }
                    }
                    OpenEntry if files.iter().all(|f| f.is_dir) => {
                        Some(t::REASON_FOLDER.to_owned())
                    }
                    OpenEntry
                        if *zone == Zone::Changes
                            && in_scope.iter().all(|f| f.kind == FileKind::Deleted) =>
                    {
                        Some(t::REASON_DELETED.to_owned())
                    }
                    _ => None,
                }
            }
            Target::Test(_) | Target::Hit(_) | Target::Editor(_) | Target::Terminal(_) => None,
            Target::Commit(c) => match id {
                LogCherryPick | LogRevert if c.parents > 1 => {
                    Some(t::REASON_MERGE_COMMIT.to_owned())
                }
                LogCherryPick | LogRevert | LogCheckout if operation => {
                    Some(t::REASON_OPERATION.to_owned())
                }
                LogCherryPick | LogRevert if no_commit => Some(t::REASON_NO_COMMIT.to_owned()),
                _ => None,
            },
            Target::Branch(b) => {
                let current = self.is_current_branch(b);
                match id {
                    BranchCheckout | BranchCheckoutRebase | BranchRebaseOnto | BranchMerge
                    | BranchPullMerge | BranchPullRebase
                        if operation =>
                    {
                        Some(t::REASON_OPERATION.to_owned())
                    }
                    BranchCheckoutRebase | BranchRebaseOnto | BranchMerge | BranchCompare
                    | BranchPullMerge | BranchPullRebase
                        if self.current_branch().is_none() =>
                    {
                        Some(t::REASON_DETACHED.to_owned())
                    }
                    BranchUpdate => {
                        let has_upstream = self
                            .branches
                            .as_ref()
                            .and_then(|l| l.find_local(&b.name))
                            .is_some_and(|br| br.tracking.is_some());
                        if !has_upstream {
                            Some(t::REASON_NO_UPSTREAM.to_owned())
                        } else if current && operation {
                            Some(t::REASON_OPERATION.to_owned())
                        } else {
                            None
                        }
                    }
                    BranchDelete if current => Some(t::REASON_CURRENT_BRANCH.to_owned()),
                    BranchDelete if b.remote => {
                        let short = b.name.split_once('/').map_or(b.name.as_str(), |(_, s)| s);
                        self.protected(short)
                            .then(|| t::REASON_PROTECTED.to_owned())
                    }
                    _ => None,
                }
            }
        }
    }

    /// Disponibilité d'une action du registre, hors menu (palette, raccourci).
    pub(super) fn v1_action_enabled(&self, id: ActionId) -> bool {
        use ActionId::*;
        match id {
            UpdateProject | Push | NewBranch | BranchesPopup | Fetch => self.git_status().is_some(),
            ContextMenuKey => true,
            // Entrées propres à une zone : seulement par son menu contextuel.
            EditCut | EditCopy | EditPaste | EditSelectAll | GotoDeclaration | FindUsages
            | TestRunNode | TestGoto | TestCompare | TestCopyName | HitOpen | HitCopyLine
            | HitExclude | TermCopy | TermPaste | TermSelectAll | TermClear | TermRename
            | TermClose | TermNewCmd | TermNewGitBash => false,
            TestRerunFailed => {
                self.tests.service.is_some() && self.tests.failures_request().is_some()
            }
            _ => self
                .current_target()
                .is_some_and(|t| self.availability(id, &t).is_none() && self.applicable(id, &t)),
        }
    }

    // ----- Exécution -------------------------------------------------------------

    /// Exécute une action v1 ; `target` : celle du menu, sinon la cible courante.
    pub(super) fn run_v1_action(&mut self, id: ActionId, target: Option<Target>) {
        use ActionId::*;
        match id {
            UpdateProject => return self.start_update(),
            Push => return self.open_push_dialog(None),
            NewBranch => {
                let label = self.current_branch().unwrap_or_else(|| "HEAD".to_owned());
                return self.open_new_branch(None, label);
            }
            BranchesPopup => return self.toggle_branch_popup(),
            Fetch => {
                if let Some((vcs, repo)) = self.vcs_pair() {
                    self.branch_popup = None;
                    vcs.fetch(&repo, FetchRequest::default());
                }
                return;
            }
            ContextMenuKey => return self.open_menu_for_focus(),
            TestRerunFailed => {
                if let Some(request) = self.tests.failures_request() {
                    self.start_run(request, false);
                }
                return;
            }
            _ => {}
        }
        let Some(target) = target.or_else(|| self.current_target()) else {
            return;
        };
        if let Some(reason) = self.availability(id, &target) {
            self.toast(reason, true, None);
            return;
        }
        // Une entrée qui porte sur le fichier de l'éditeur ou de la recherche.
        let target = if native_to(id, &target) {
            target
        } else {
            file_view(&target).unwrap_or(target)
        };
        match target {
            Target::Files { files, zone } => self.run_files_action(id, zone, files),
            Target::Commit(commit) => self.run_commit_action(id, commit),
            Target::Branch(branch) => self.run_branch_action(id, branch),
            Target::Editor(_) => self.run_editor_action(id),
            Target::Test(test) => self.run_test_action(id, test),
            Target::Hit(hit) => self.run_hit_action(id, hit),
            Target::Terminal(term) => self.run_terminal_action(id, term),
        }
    }

    fn run_editor_action(&mut self, id: ActionId) {
        use ActionId::*;
        let ctx = self.ctx.clone();
        let Some(editor) = self.editors.active_editor_mut() else {
            return;
        };
        match id {
            EditCopy => {
                if let Some(text) = editor.document.copy_text() {
                    ctx.copy_text(text);
                }
            }
            EditCut => {
                if let Some(text) = editor.document.cut() {
                    ctx.copy_text(text);
                }
            }
            EditPaste => {
                // Le texte arrive en `Event::Paste` à l'éditeur qui a le focus.
                editor.view.request_focus();
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            EditSelectAll => editor.document.select_all(),
            _ => {}
        }
        self.focus_editor();
    }

    fn run_test_action(&mut self, id: ActionId, test: TestRef) {
        use ActionId::*;
        match id {
            TestRunNode => {
                if let Some(request) = self.tests.request_for_node(test.id) {
                    self.start_run(request, false);
                }
            }
            TestRerunFailed => {
                if let Some(request) = self.tests.failures_request() {
                    self.start_run(request, false);
                }
            }
            TestGoto => {
                if let Some(location) = self.tests.location_of(test.id) {
                    self.run_tests_intents(vec![crate::testsview::TestsIntent::Open(location)]);
                }
            }
            TestCompare => {
                if let Some(intent) = self.tests.compare_intent() {
                    self.run_tests_intents(vec![intent]);
                }
            }
            TestCopyName => {
                let name = self.tests.copy_name(test.id);
                self.ctx.copy_text(name);
                self.toast(t::copied(1), false, None);
            }
            _ => {}
        }
    }

    fn run_hit_action(&mut self, id: ActionId, hit: HitRef) {
        use ActionId::*;
        match id {
            HitOpen => {
                let goto = Goto {
                    position: TextPosition::new(hit.line, 0),
                    end: None,
                };
                self.find_open = false;
                self.open_file(hit.path, Some(goto));
            }
            HitCopyLine => {
                self.ctx.copy_text(hit.text.trim_end().to_owned());
                self.toast(t::copied_line(), false, None);
            }
            HitExclude => {
                self.toast(t::hit_excluded(&file_name_of(&hit.path)), false, None);
                self.find.exclude_file(&hit.path);
            }
            _ => {}
        }
    }

    fn run_terminal_action(&mut self, id: ActionId, term: crate::target::TermRef) {
        use ActionId::*;
        let ctx = self.ctx.clone();
        self.tools.bottom = BottomTool::Terminal;
        self.tools.bottom_open = true;
        let Some(terminal) = &mut self.terminal else {
            return;
        };
        match id {
            TermCopy => {
                if let Some(text) = terminal.selection_text() {
                    ctx.copy_text(text);
                }
            }
            TermPaste => terminal.paste(&ctx),
            TermSelectAll => terminal.select_all(),
            TermClear => terminal.clear_history(),
            TermRename => terminal.begin_rename(term.tab),
            TermClose => terminal.close_tab(term.tab),
            TermNewCmd => terminal.open_kind(julie_terminal::ProfileKind::Cmd),
            TermNewGitBash => terminal.open_kind(julie_terminal::ProfileKind::GitBash),
            _ => {}
        }
        terminal.focus();
    }

    fn scope_paths(&self, files: &[FileRef], keep: impl Fn(&FileStatus) -> bool) -> Vec<PathBuf> {
        self.git_status()
            .map(|s| {
                scope(s, files)
                    .into_iter()
                    .filter(|f| keep(f))
                    .map(|f| f.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn run_files_action(&mut self, id: ActionId, zone: Zone, files: Vec<FileRef>) {
        use ActionId::*;
        let Some(first) = files.first().cloned() else {
            return;
        };
        let root = self.project_root();
        match id {
            GitAdd => {
                let paths = self.scope_paths(&files, can_add);
                if let (Some((vcs, repo)), false) = (self.vcs_pair(), paths.is_empty()) {
                    vcs.stage(&repo, paths);
                }
            }
            GitUnstage => {
                let paths = self.scope_paths(&files, can_unstage);
                if let (Some((vcs, repo)), false) = (self.vcs_pair(), paths.is_empty()) {
                    vcs.unstage(&repo, paths);
                }
            }
            GitDiscard => self.open_discard(&files),
            GitShowDiff => {
                let repo_path = self
                    .git_status()
                    .and_then(|s| s.find(&first.path).map(|f| f.repo_path.clone()));
                if let Some(repo_path) = repo_path {
                    self.git.select_path(&repo_path);
                    let mut intents = Vec::new();
                    self.git.open_diff_window(&mut intents);
                    self.run_git_intents(intents);
                }
            }
            GitFileHistory => {
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git.show_file_history(first.path);
            }
            IgnoreFile | IgnoreFolder | IgnoreExtension => {
                let rule = match id {
                    IgnoreFile => IgnoreRule::File(first.path.clone()),
                    IgnoreFolder => IgnoreRule::Folder(first.path.clone()),
                    _ => IgnoreRule::Extension(
                        first
                            .path
                            .extension()
                            .map(|e| e.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    ),
                };
                if let Some((vcs, repo)) = self.vcs_pair() {
                    vcs.add_to_gitignore(&repo, rule);
                }
            }
            OpenEntry => {
                for file in files.iter().filter(|f| !f.is_dir) {
                    self.open_file(file.path.clone(), None);
                }
            }
            CopyPath | CopyRelativePath | CopyName => {
                let lines: Vec<String> = files
                    .iter()
                    .map(|f| match id {
                        CopyPath => f.path.display().to_string(),
                        CopyRelativePath => root
                            .as_ref()
                            .and_then(|r| f.path.strip_prefix(r).ok())
                            .map_or_else(
                                || f.path.display().to_string(),
                                |r| r.to_string_lossy().replace('\\', "/"),
                            ),
                        _ => file_name_of(&f.path),
                    })
                    .collect();
                self.ctx.copy_text(lines.join("\n"));
                self.toast(t::copied(lines.len()), false, None);
            }
            RevealInExplorer => {
                if !self.scenario_active {
                    let _ = std::process::Command::new("explorer")
                        .arg(format!("/select,{}", first.path.display()))
                        .spawn();
                }
            }
            OpenTerminalHere => {
                let dir = if first.is_dir {
                    first.path.clone()
                } else {
                    first
                        .path
                        .parent()
                        .map_or_else(|| first.path.clone(), Path::to_path_buf)
                };
                self.tools.bottom = BottomTool::Terminal;
                self.tools.bottom_open = true;
                if let Some(terminal) = &mut self.terminal {
                    terminal.open_default_in(dir);
                }
            }
            FindInFolder => {
                let dir = if first.is_dir {
                    first.path.clone()
                } else {
                    first
                        .path
                        .parent()
                        .map_or_else(|| first.path.clone(), Path::to_path_buf)
                };
                self.close_floating();
                self.find_open = true;
                self.find.prepare(None, Some(dir));
                self.find.refresh_now();
            }
            TreeNewFile | TreeNewFolder => {
                let dir = if first.is_dir {
                    first.path.clone()
                } else {
                    first
                        .path
                        .parent()
                        .map_or_else(|| first.path.clone(), Path::to_path_buf)
                };
                self.open_new_item(dir, id == TreeNewFolder);
            }
            TreeRename => self.open_rename_item(first.path.clone()),
            TreeDelete => {
                let paths: Vec<PathBuf> = if zone == Zone::Tree {
                    files.iter().map(|f| f.path.clone()).collect()
                } else {
                    self.scope_paths(&files, |f| f.kind == FileKind::Untracked)
                };
                if !paths.is_empty() {
                    self.open_delete_items(paths);
                }
            }
            RunTestsHere => {
                let request = match tests_path_kind(&first) {
                    Some(RunKind::Config) => RunRequest::all(),
                    _ => RunRequest::path(first.path.clone()),
                };
                self.start_run(request, false);
            }
            CloseTab => {
                if let Some(index) = self.editors.index_of(&first.path) {
                    self.request_close(index);
                }
            }
            TabCloseOthers | TabCloseAll | TabCloseUnmodified => {
                let paths: Vec<PathBuf> = self
                    .editors
                    .items
                    .iter()
                    .filter(|e| match id {
                        TabCloseOthers => e.path() != Some(first.path.as_path()),
                        TabCloseUnmodified => !e.document.is_modified(),
                        _ => true,
                    })
                    .filter_map(|e| e.path().map(Path::to_path_buf))
                    .collect();
                self.close_tabs(paths);
            }
            _ => {
                let _ = zone;
            }
        }
    }

    /// Ferme ces onglets : les non modifiés tout de suite, la question une seule fois
    /// (avec la liste) pour les modifiés.
    fn close_tabs(&mut self, paths: Vec<PathBuf>) {
        let mut modified = Vec::new();
        for path in paths {
            let Some(index) = self.editors.index_of(&path) else {
                continue;
            };
            if self.editors.items[index].document.is_modified() {
                modified.push(path);
            } else {
                self.editors.close(index);
            }
        }
        if !modified.is_empty() {
            self.dialog = Some(Dialog::CloseMany(modified));
        }
    }

    fn open_discard(&mut self, files: &[FileRef]) {
        let Some(status) = self.git_status() else {
            return;
        };
        let affected: Vec<&FileStatus> = scope(status, files)
            .into_iter()
            .filter(|f| can_discard(f))
            .collect();
        let mut items = Vec::new();
        let mut paths = Vec::new();
        let mut added = 0;
        for f in &affected {
            let name = f.repo_path.clone();
            let what = match f.kind {
                FileKind::Added | FileKind::Copied => {
                    added += 1;
                    t::DISCARD_ADDED
                }
                FileKind::Renamed => {
                    added += 1;
                    t::DISCARD_RENAMED
                }
                FileKind::Deleted => t::DISCARD_DELETED,
                _ => t::DISCARD_MODIFIED,
            };
            items.push((name, what.to_owned()));
            paths.push(f.path.clone());
            if let Some(original) = &f.original_path {
                paths.push(original.clone());
            }
        }
        let unsaved: Vec<String> = paths
            .iter()
            .filter_map(|p| {
                self.editors
                    .index_of(p)
                    .filter(|i| self.editors.items[*i].document.is_modified())
                    .map(|_| file_name_of(p))
            })
            .collect();
        self.git_dialog = Some(GitDialog::Discard(gd::Discard {
            paths,
            items,
            added,
            unsaved,
            delete_added: false,
        }));
    }

    fn run_commit_action(&mut self, id: ActionId, commit: CommitRef) {
        use ActionId::*;
        let pair = self.vcs_pair();
        match id {
            LogNewBranch => self.open_new_branch(Some(commit.id), commit.short),
            LogCheckout => {
                self.git_dialog = Some(GitDialog::CheckoutCommit(gd::CheckoutCommit {
                    id: commit.id,
                    short: commit.short,
                    subject: commit.subject,
                }));
            }
            LogCherryPick => {
                if let Some((vcs, repo)) = pair {
                    vcs.cherry_pick(&repo, vec![commit.id], LocalChanges::Keep);
                }
            }
            LogRevert => {
                if let Some((vcs, repo)) = pair {
                    vcs.revert_commits(&repo, vec![commit.id], LocalChanges::Keep);
                }
            }
            LogReset => self.open_reset(commit.id, commit.short, commit.subject),
            CopyHash => {
                self.ctx.copy_text(commit.id);
                self.toast(t::copied(1), false, None);
            }
            CopyShortHash => {
                self.ctx.copy_text(commit.short);
                self.toast(t::copied(1), false, None);
            }
            CopyMessage => {
                self.ctx.copy_text(commit.subject);
                self.toast(t::copied(1), false, None);
            }
            _ => {}
        }
    }

    fn run_branch_action(&mut self, id: ActionId, branch: BranchRef) {
        use ActionId::*;
        let pair = self.vcs_pair();
        let current = self.current_branch().unwrap_or_default();
        let short = branch
            .name
            .split_once('/')
            .map_or(branch.name.clone(), |(_, s)| s.to_owned());
        let remote_name = branch
            .name
            .split_once('/')
            .map(|(r, _)| r.to_owned())
            .unwrap_or_default();
        match id {
            BranchCheckout => {
                let target = if branch.remote {
                    SwitchTarget::Remote(branch.name.clone())
                } else {
                    SwitchTarget::Branch(branch.name.clone())
                };
                self.request_switch(target, LocalChanges::Keep);
            }
            BranchNewFrom => self.open_new_branch(Some(branch.name.clone()), branch.name),
            BranchCheckoutRebase => {
                if let Some((vcs, repo)) = pair {
                    vcs.rebase(
                        &repo,
                        RebaseRequest {
                            onto: current,
                            branch: Some(branch.name),
                            local_changes: LocalChanges::Stash,
                        },
                    );
                }
            }
            BranchCompare => {
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git
                    .show_log_range(format!("{current}..{}", branch.name));
            }
            BranchRebaseOnto => {
                if let Some((vcs, repo)) = pair {
                    vcs.rebase(
                        &repo,
                        RebaseRequest {
                            onto: branch.name,
                            branch: None,
                            local_changes: LocalChanges::Stash,
                        },
                    );
                }
            }
            BranchMerge => {
                if let Some((vcs, repo)) = pair {
                    vcs.merge(
                        &repo,
                        MergeRequest {
                            revision: branch.name,
                            ..MergeRequest::default()
                        },
                    );
                }
            }
            BranchUpdate => {
                if branch.name == current {
                    self.start_update();
                } else if let Some((vcs, repo)) = pair {
                    vcs.fast_forward_branch(&repo, branch.name);
                }
            }
            BranchPullMerge | BranchPullRebase => {
                if let Some((vcs, repo)) = pair {
                    vcs.pull(
                        &repo,
                        PullRequest {
                            remote: remote_name,
                            branch: short,
                            mode: if id == BranchPullMerge {
                                PullMode::Merge
                            } else {
                                PullMode::Rebase
                            },
                            local_changes: LocalChanges::Stash,
                        },
                    );
                }
            }
            BranchPush => self.open_push_dialog(Some(branch.name)),
            BranchRename => self.open_rename(branch.name),
            BranchDelete => {
                if branch.remote {
                    self.git_dialog = Some(GitDialog::DeleteRemote(gd::DeleteRemote {
                        remote: remote_name,
                        branch: short,
                    }));
                } else {
                    let upstream = self
                        .branches
                        .as_ref()
                        .and_then(|l| l.find_local(&branch.name))
                        .and_then(|b| b.tracking.as_ref().map(|t| t.upstream.clone()));
                    self.git_dialog = Some(GitDialog::DeleteBranch(gd::DeleteBranch {
                        name: branch.name,
                        upstream,
                        also_remote: false,
                    }));
                }
            }
            _ => {}
        }
        let _ = OperationKind::Merge;
    }

    // ----- Mode scénario ---------------------------------------------------------

    /// `menu.open <zone> <argument>`.
    pub(super) fn scenario_open_menu(&mut self, arg: &str) -> Result<(), String> {
        let (zone, rest) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
        let rest = rest.trim();
        let root = self.project_root().ok_or("pas de projet ouvert")?;
        let (target, pos) = match zone {
            "tree" => {
                // Plusieurs éléments séparés par des virgules : sélection multiple.
                let mut files = Vec::new();
                for part in rest.split(',').map(str::trim).filter(|p| !p.is_empty() || rest.is_empty()) {
                    let path = if part.is_empty() || part == "." {
                        root.clone()
                    } else {
                        root.join(part.replace('/', "\\"))
                    };
                    self.tree.expand_path(&path);
                    let is_dir = path.is_dir();
                    files.push(FileRef { path, is_dir });
                }
                if files.is_empty() {
                    files.push(FileRef {
                        path: root.clone(),
                        is_dir: true,
                    });
                }
                self.tree
                    .select_many(files.iter().map(|f| f.path.clone()).collect());
                self.tools.project_open = true;
                (
                    Target::Files {
                        zone: Zone::Tree,
                        files,
                    },
                    egui::pos2(150.0, 150.0),
                )
            }
            "editor" => {
                if self.editors.active_editor().is_none() {
                    return Err("aucun éditeur ouvert".to_owned());
                }
                (self.editor_target(), egui::pos2(520.0, 240.0))
            }
            "test" => {
                self.tools.bottom = BottomTool::Tests;
                self.tools.bottom_open = true;
                if !self.tests.select_label(rest) {
                    return Err(format!("nœud de test introuvable : {rest}"));
                }
                let test = self.tests.selected_ref().ok_or("aucun arbre de tests")?;
                (Target::Test(test), egui::pos2(300.0, 460.0))
            }
            "hit" => {
                let n = rest.parse::<usize>().map_err(|_| "numéro de résultat attendu")?;
                let hit = self
                    .find
                    .hit_target(n.saturating_sub(1))
                    .ok_or_else(|| format!("résultat {n} absent"))?;
                (Target::Hit(hit), egui::pos2(500.0, 300.0))
            }
            "terminal" => {
                self.tools.bottom = BottomTool::Terminal;
                self.tools.bottom_open = true;
                let on_tab = rest == "tab";
                (
                    self.terminal_target(on_tab).ok_or("pas de terminal")?,
                    egui::pos2(500.0, 460.0),
                )
            }
            "changes" => {
                let status = self.git.status().cloned().ok_or("état Git inconnu")?;
                let mut files = Vec::new();
                for wanted in rest.split(',').map(str::trim).filter(|w| !w.is_empty()) {
                    let found = status
                        .files
                        .iter()
                        .find(|f| f.repo_path == wanted.replace('\\', "/"))
                        .ok_or_else(|| format!("fichier absent de la liste Git : {wanted}"))?;
                    files.push(FileRef {
                        path: found.path.clone(),
                        is_dir: false,
                    });
                }
                if files.is_empty() {
                    return Err("`menu.open changes <fichier>[,<fichier>…]`".to_owned());
                }
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git.tab = GitTab::Changes;
                if let Some(first) = status.files.iter().find(|f| f.path == files[0].path) {
                    self.git.select_path(&first.repo_path);
                }
                (
                    Target::Files {
                        zone: Zone::Changes,
                        files,
                    },
                    egui::pos2(160.0, 330.0),
                )
            }
            "tab" => {
                let index = match rest.parse::<usize>() {
                    Ok(n) => n.saturating_sub(1),
                    Err(_) => self
                        .editors
                        .items
                        .iter()
                        .position(|e| e.path().is_some_and(|p| p.to_string_lossy().contains(rest)))
                        .ok_or_else(|| format!("onglet introuvable : {rest}"))?,
                };
                let path = self
                    .editors
                    .items
                    .get(index)
                    .and_then(|e| e.path().map(Path::to_path_buf))
                    .ok_or("onglet sans fichier")?;
                (
                    Target::Files {
                        zone: Zone::Tab,
                        files: vec![FileRef {
                            path,
                            is_dir: false,
                        }],
                    },
                    egui::pos2(300.0, 62.0),
                )
            }
            "log" => {
                let n = rest
                    .parse::<usize>()
                    .map_err(|_| "numéro de commit attendu")?;
                let mut intents = Vec::new();
                if !self.git.select_log(n.saturating_sub(1), &mut intents) {
                    return Err(format!("commit {n} absent du journal"));
                }
                self.run_git_intents(intents);
                let commit = self
                    .git
                    .log
                    .commit_ref(n.saturating_sub(1))
                    .ok_or("commit introuvable")?;
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git.tab = GitTab::Log;
                (Target::Commit(commit), egui::pos2(520.0, 380.0))
            }
            "branch" => {
                let remote = self
                    .branches
                    .as_ref()
                    .is_some_and(|l| l.find_remote(rest).is_some());
                (
                    Target::Branch(BranchRef {
                        name: rest.to_owned(),
                        remote,
                    }),
                    egui::pos2(760.0, 220.0),
                )
            }
            other => {
                return Err(format!(
                    "zone inconnue : « {other} » (tree, changes, tab, log, branch, editor, test, hit, terminal)"
                ))
            }
        };
        self.open_menu(target, pos, false);
        if self.ctx_menu.is_none() {
            return Err("aucune entrée de menu pour cette cible".to_owned());
        }
        Ok(())
    }

    /// `menu.choose <clé d'action>` : choisit l'entrée (sous-menus compris).
    pub(super) fn scenario_choose(&mut self, key: &str) -> Result<(), String> {
        let menu = self.ctx_menu.as_ref().ok_or("aucun menu ouvert")?;
        let (_, _, entry) = menu
            .find(key)
            .ok_or_else(|| format!("entrée inconnue dans ce menu : « {key} »"))?;
        if let Some(reason) = &entry.disabled {
            return Err(format!("entrée « {key} » grisée : {reason}"));
        }
        let id = entry
            .id
            .ok_or("cette entrée est un sous-menu : `menu.expand`")?;
        self.finish_menu(id);
        Ok(())
    }
}
