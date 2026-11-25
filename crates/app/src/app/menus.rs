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
use crate::target::{CommitRef, FileRef, Target, Zone};
use crate::texts_v1 as tv;

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
            self.tree.selected_ref().map(|f| Target::Files {
                zone: Zone::Tree,
                files: vec![f],
            })
        };
        let changes = || {
            let status = self.git.status()?;
            let selected = self.git.selected.as_ref()?;
            status.find(selected).map(|f| Target::Files {
                zone: Zone::Changes,
                files: vec![FileRef {
                    path: f.path.clone(),
                    is_dir: false,
                }],
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
        let Some(target) = self.current_target() else {
            return;
        };
        let screen = self.ctx.content_rect();
        let pos = match &target {
            Target::Branch(_) => egui::pos2(screen.right() - 440.0, 140.0),
            Target::Files {
                zone: Zone::Tree, ..
            } => egui::pos2(120.0, 160.0),
            Target::Files {
                zone: Zone::Tab, ..
            } => egui::pos2(260.0, 60.0),
            Target::Files { .. } | Target::Commit(_) => {
                egui::pos2(screen.left() + 160.0, screen.bottom() - 260.0)
            }
        };
        self.open_menu(target, pos, true);
    }

    pub(super) fn collect_menu_requests(&mut self) {
        let request = self
            .tree
            .menu_request
            .take()
            .or_else(|| self.editors.menu_request.take())
            .or_else(|| self.git.menu_request.take());
        if let Some(request) = request {
            self.close_floating_keep_popup();
            self.open_menu(request.target, request.pos, request.by_keyboard);
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
            self.run_v1_action(id, Some(target));
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
                tv::MENU_COPY_PATH.to_owned(),
                vec![Action(CopyPath), Action(CopyRelativePath), Action(CopyName)],
            )
        };
        match target {
            Target::Files {
                zone: Zone::Tree, ..
            } => vec![
                Action(OpenEntry),
                Sep,
                Sub(
                    "sub.git",
                    tv::MENU_GIT.to_owned(),
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
                copy(),
                Action(RevealInExplorer),
                Action(OpenTerminalHere),
                Action(FindInFolder),
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
                    tv::MENU_IGNORE.to_owned(),
                    vec![
                        Action(IgnoreFile),
                        Action(IgnoreFolder),
                        Action(IgnoreExtension),
                    ],
                ),
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
                    tv::MENU_GIT.to_owned(),
                    vec![
                        Action(GitShowDiff),
                        Action(GitDiscard),
                        Action(GitFileHistory),
                    ],
                ),
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
            Target::Commit(_) => true,
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
            (BranchCheckoutRebase, Target::Branch(_)) => tv::label_checkout_rebase(&current),
            (BranchCompare, Target::Branch(_)) => tv::label_compare(&current),
            (BranchRebaseOnto, Target::Branch(b)) => tv::label_rebase_onto(&current, &b.name),
            (BranchMerge, Target::Branch(b)) => tv::label_merge_into(&b.name, &current),
            (BranchPullMerge, Target::Branch(_)) => tv::label_pull_merge(&current),
            (BranchPullRebase, Target::Branch(_)) => tv::label_pull_rebase(&current),
            (BranchNewFrom, Target::Branch(b)) => tv::label_new_from(&b.name),
            (IgnoreExtension, Target::Files { files, .. }) => files
                .first()
                .and_then(|f| f.path.extension())
                .map_or(registry_label, |e| {
                    tv::label_ignore_ext(&e.to_string_lossy())
                }),
            (
                OpenEntry,
                Target::Files {
                    zone: Zone::Changes,
                    ..
                },
            ) => tv::LABEL_OPEN_FILE.to_owned(),
            (CloseTab, _) => tv::LABEL_CLOSE_TAB.to_owned(),
            _ => registry_label,
        }
    }

    /// `None` : l'entrée est active ; sinon la raison du grisé (infobulle).
    pub(super) fn availability(&self, id: ActionId, target: &Target) -> Option<String> {
        use ActionId::*;
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
                    tv::REASON_NOT_IN_REPO
                } else {
                    tv::REASON_GIT_LOADING
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
                        Some(tv::REASON_NOTHING_TO_ADD.to_owned())
                    }
                    GitUnstage if !in_scope.iter().any(|f| can_unstage(f)) => {
                        Some(tv::REASON_NOTHING_STAGED.to_owned())
                    }
                    GitDiscard => {
                        if in_scope.iter().any(|f| f.group() == Group::Conflicted) {
                            Some(tv::REASON_RESOLVE_CONFLICT.to_owned())
                        } else if !in_scope.iter().any(|f| can_discard(f)) {
                            Some(tv::REASON_NOTHING_TO_DISCARD.to_owned())
                        } else {
                            None
                        }
                    }
                    GitShowDiff => {
                        if files.len() != 1 || files[0].is_dir {
                            Some(tv::REASON_ONE_FILE.to_owned())
                        } else if in_scope.is_empty() {
                            Some(tv::REASON_NO_CHANGE.to_owned())
                        } else if in_scope[0].group() == Group::Conflicted {
                            Some(tv::REASON_RESOLVE_CONFLICT.to_owned())
                        } else {
                            None
                        }
                    }
                    GitFileHistory => {
                        if files.len() != 1 || files[0].is_dir {
                            Some(tv::REASON_ONE_FILE.to_owned())
                        } else if in_scope.iter().any(|f| f.kind == FileKind::Untracked) {
                            Some(tv::REASON_UNTRACKED.to_owned())
                        } else if no_commit {
                            Some(tv::REASON_NO_COMMIT.to_owned())
                        } else {
                            None
                        }
                    }
                    IgnoreFile | IgnoreFolder | IgnoreExtension => {
                        if files.len() != 1 {
                            Some(tv::REASON_ONE_FILE.to_owned())
                        } else if in_scope.iter().any(|f| f.kind != FileKind::Untracked) {
                            Some(tv::REASON_ALREADY_TRACKED.to_owned())
                        } else {
                            None
                        }
                    }
                    TabCloseOthers if self.editors.items.len() <= 1 => {
                        Some(tv::REASON_ONE_TAB.to_owned())
                    }
                    TabCloseUnmodified
                        if self.editors.items.iter().all(|e| e.document.is_modified()) =>
                    {
                        Some(tv::REASON_NO_UNMODIFIED_TAB.to_owned())
                    }
                    OpenEntry if files.iter().all(|f| f.is_dir) => {
                        Some(tv::REASON_FOLDER.to_owned())
                    }
                    OpenEntry
                        if *zone == Zone::Changes
                            && in_scope.iter().all(|f| f.kind == FileKind::Deleted) =>
                    {
                        Some(tv::REASON_DELETED.to_owned())
                    }
                    _ => None,
                }
            }
            Target::Commit(c) => match id {
                LogCherryPick | LogRevert if c.parents > 1 => {
                    Some(tv::REASON_MERGE_COMMIT.to_owned())
                }
                LogCherryPick | LogRevert | LogCheckout if operation => {
                    Some(tv::REASON_OPERATION.to_owned())
                }
                LogCherryPick | LogRevert if no_commit => Some(tv::REASON_NO_COMMIT.to_owned()),
                _ => None,
            },
            Target::Branch(b) => {
                let current = self.is_current_branch(b);
                match id {
                    BranchCheckout | BranchCheckoutRebase | BranchRebaseOnto | BranchMerge
                    | BranchPullMerge | BranchPullRebase
                        if operation =>
                    {
                        Some(tv::REASON_OPERATION.to_owned())
                    }
                    BranchCheckoutRebase | BranchRebaseOnto | BranchMerge | BranchCompare
                    | BranchPullMerge | BranchPullRebase
                        if self.current_branch().is_none() =>
                    {
                        Some(tv::REASON_DETACHED.to_owned())
                    }
                    BranchUpdate => {
                        let has_upstream = self
                            .branches
                            .as_ref()
                            .and_then(|l| l.find_local(&b.name))
                            .is_some_and(|br| br.tracking.is_some());
                        if !has_upstream {
                            Some(tv::REASON_NO_UPSTREAM.to_owned())
                        } else if current && operation {
                            Some(tv::REASON_OPERATION.to_owned())
                        } else {
                            None
                        }
                    }
                    BranchDelete if current => Some(tv::REASON_CURRENT_BRANCH.to_owned()),
                    BranchDelete if b.remote => {
                        let short = b.name.split_once('/').map_or(b.name.as_str(), |(_, s)| s);
                        self.protected(short)
                            .then(|| tv::REASON_PROTECTED.to_owned())
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
            _ => {}
        }
        let Some(target) = target.or_else(|| self.current_target()) else {
            return;
        };
        if let Some(reason) = self.availability(id, &target) {
            self.toast(reason, true, None);
            return;
        }
        match target {
            Target::Files { files, zone } => self.run_files_action(id, zone, files),
            Target::Commit(commit) => self.run_commit_action(id, commit),
            Target::Branch(branch) => self.run_branch_action(id, branch),
        }
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
                self.toast(tv::copied(lines.len()), false, None);
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
                    tv::DISCARD_ADDED
                }
                FileKind::Renamed => {
                    added += 1;
                    tv::DISCARD_RENAMED
                }
                FileKind::Deleted => tv::DISCARD_DELETED,
                _ => tv::DISCARD_MODIFIED,
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
                self.toast(tv::copied(1), false, None);
            }
            CopyShortHash => {
                self.ctx.copy_text(commit.short);
                self.toast(tv::copied(1), false, None);
            }
            CopyMessage => {
                self.ctx.copy_text(commit.subject);
                self.toast(tv::copied(1), false, None);
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
                let path = if rest.is_empty() || rest == "." {
                    root.clone()
                } else {
                    root.join(rest.replace('/', "\\"))
                };
                self.tree.expand_path(&path);
                self.tools.project_open = true;
                let is_dir = path.is_dir();
                (
                    Target::Files {
                        zone: Zone::Tree,
                        files: vec![FileRef { path, is_dir }],
                    },
                    egui::pos2(150.0, 150.0),
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
                    "zone inconnue : « {other} » (tree, changes, tab, log, branch)"
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
