//! Les flux Git de la v1 côté interface : branches (fenêtre surgissante, bascule,
//! création, renommage, suppression), mise à jour du projet, push, fusion et
//! rebase, réinitialisation, annulation des modifications, copies de sécurité.
//!
//! Ce fichier ne contient **aucune règle Git** : il traduit des gestes (boutons
//! des dialogues, actions du registre) en appels à `julie-vcs` et les événements
//! publiés en notifications, dialogues et ouverture des vues existantes.

use super::*;

use julie_vcs::{
    Action as VcsAction, CreateBranchRequest, IntegrationMethod, LocalChanges, OperationOutcome,
    OverwriteConflict, PushForce, PushRejection, PushRequest, PushTarget, Repository, ResetMode,
    StashRestore, SwitchOutcome, SwitchRequest, SwitchTarget, UpdateRequest, VcsError,
};

use crate::branchui::{BranchPopup, PopupOut, WidgetInfo, WidgetKind};
use crate::gitdialogs::{self as gd, Dialog as GitDialog, ResetUiMode};
use crate::safety::AfterCopy;
use crate::target::Target;
use crate::texts as t;

/// Ce qu'il faut rejouer si la bascule est refusée puis autorisée avec mise de côté.
pub(super) enum Pending {
    Switch(SwitchTarget),
    Create(CreateBranchRequest),
}

/// Le bouton de la notification de la barre d'état.
#[derive(Clone)]
pub(super) enum ToastAction {
    RestoreBranch {
        name: String,
        tip: String,
    },
    RestoreCopy(String),
    /// Ouvrir un fichier de réglages illisible, à la ligne de l'erreur.
    OpenFile(PathBuf, Option<u32>),
}

/// Les actions Git qui écrivent : un échec de Git y est montré tel quel dans une fenêtre.
fn shows_output(action: VcsAction) -> bool {
    use VcsAction::*;
    matches!(
        action,
        Switch
            | CreateBranch
            | RenameBranch
            | DeleteBranch
            | RestoreBranch
            | DeleteRemoteBranch
            | Fetch
            | FastForwardBranch
            | Update
            | Pull
            | Push
            | PushPreview
            | Merge
            | Rebase
            | CherryPick
            | Revert
            | Skip
            | ResetPreview
            | Reset
            | Stage
            | Unstage
            | Discard
            | Ignore
    )
}

impl JulieStormApp {
    pub(super) fn vcs_pair(&self) -> Option<(Vcs, Repository)> {
        self.project
            .as_ref()
            .and_then(|p| p.repo.clone().map(|r| (p.vcs.clone(), r)))
    }

    pub(super) fn git_status(&self) -> Option<&julie_vcs::Status> {
        match self.project.as_ref().map(|p| &p.git) {
            Some(GitInfo::Repository(status)) => Some(status),
            _ => None,
        }
    }

    pub(super) fn project_root(&self) -> Option<PathBuf> {
        self.project
            .as_ref()
            .map(|p| p.project.root().to_path_buf())
    }

    /// Branche courante (nom), si HEAD est sur une branche.
    pub(super) fn current_branch(&self) -> Option<String> {
        match &self.git_status()?.head {
            julie_vcs::Head::Branch { name, .. } => Some(name.clone()),
            julie_vcs::Head::Detached { .. } => None,
        }
    }

    pub(super) fn protected(&self, branch: &str) -> bool {
        self.project
            .as_ref()
            .is_some_and(|p| p.vcs.settings().is_protected(branch))
    }

    /// Une information dans la barre d'état, avec un bouton éventuel.
    pub(super) fn toast(
        &mut self,
        text: impl Into<String>,
        error: bool,
        action: Option<ToastAction>,
    ) {
        self.say(text, error);
        if let Some(message) = &mut self.message {
            message.action = action;
        }
    }

    pub(super) fn open_message(
        &mut self,
        title: String,
        text: String,
        error: bool,
        extra: Vec<(&'static str, String)>,
    ) {
        self.git_dialog = Some(GitDialog::Message(gd::Message {
            title,
            text,
            error,
            extra,
        }));
    }

    pub(super) fn show_git_changes(&mut self) {
        self.tools.bottom = BottomTool::Git;
        self.tools.bottom_open = true;
        self.git.tab = GitTab::Changes;
    }

    // ----- Widget et fenêtre surgissante ----------------------------------------

    pub(super) fn load_branches(&mut self) {
        if let Some((vcs, repo)) = self.vcs_pair() {
            vcs.load_branches(&repo);
        }
    }

    pub(super) fn toggle_branch_popup(&mut self) {
        if self.branch_popup.is_some() {
            self.branch_popup = None;
            self.ctx_menu = None;
            self.focus_editor();
            return;
        }
        if self.vcs_pair().is_none() {
            return;
        }
        self.close_floating();
        let mut popup = BranchPopup::new();
        if let Some(list) = &self.branches {
            popup.rebuild(list);
        }
        self.branch_popup = Some(popup);
        self.load_branches();
    }

    pub(super) fn close_floating(&mut self) {
        self.goto = None;
        self.find_open = false;
        self.palette = None;
        self.ctx_menu = None;
    }

    pub(super) fn widget_info(&self) -> Option<WidgetInfo> {
        let project = self.project.as_ref()?;
        Some(match &project.git {
            GitInfo::Unknown => return None,
            GitInfo::NoRepository => WidgetInfo {
                text: t::WIDGET_NO_REPO.to_owned(),
                ahead: 0,
                behind: 0,
                kind: WidgetKind::NoRepository,
            },
            GitInfo::Repository(status) => {
                let (ahead, behind) = status
                    .upstream
                    .as_ref()
                    .and_then(|u| u.ahead_behind)
                    .unwrap_or((0, 0));
                let (text, kind) = match (&status.operation, &status.head) {
                    (Some(operation), _) => (
                        t::widget_operation(operation.kind, operation.step),
                        WidgetKind::Operation,
                    ),
                    (None, julie_vcs::Head::Detached { commit }) => (
                        t::widget_detached(&commit.chars().take(7).collect::<String>()),
                        WidgetKind::Detached,
                    ),
                    (None, julie_vcs::Head::Branch { name, commit }) => (
                        name.clone(),
                        if commit.is_none() {
                            WidgetKind::Unborn
                        } else {
                            WidgetKind::Normal
                        },
                    ),
                };
                WidgetInfo {
                    text,
                    ahead,
                    behind,
                    kind,
                }
            }
        })
    }

    /// Infobulle du widget : branche, amont, avance et retard, dernier fetch.
    pub(super) fn widget_tooltip(&self) -> String {
        let Some(status) = self.git_status() else {
            return String::new();
        };
        let branch = match &status.head {
            julie_vcs::Head::Branch { name, .. } => name.clone(),
            julie_vcs::Head::Detached { commit } => {
                t::widget_detached(&commit.chars().take(7).collect::<String>())
            }
        };
        let upstream = status.upstream.as_ref();
        t::widget_tooltip(
            &branch,
            upstream.map(|u| u.name.as_str()),
            upstream.and_then(|u| u.ahead_behind),
            self.last_fetch.map(|t| t.elapsed().as_secs()),
        )
    }

    pub(super) fn draw_branch_popup(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(popup) = &mut self.branch_popup else {
            return;
        };
        let list = self.branches.clone().unwrap_or_default();
        let anchor = self
            .branch_widget
            .map_or(egui::pos2(ctx.content_rect().right() - 10.0, 32.0), |r| {
                egui::pos2(r.right(), r.bottom() + 3.0)
            });
        let out = popup.show(
            ctx,
            p,
            &list,
            &self.registry,
            anchor,
            self.ctx_menu.is_some(),
        );
        match out {
            PopupOut::None => {}
            PopupOut::Close => {
                self.branch_popup = None;
                self.focus_editor();
            }
            PopupOut::Run(id) => {
                self.branch_popup = None;
                self.run_action(id);
            }
            PopupOut::Menu { branch, anchor } => {
                self.open_menu(Target::Branch(branch), anchor, false);
            }
        }
    }

    // ----- Événements ------------------------------------------------------------

    pub(super) fn on_vcs_v1(&mut self, event: &VcsEvent) {
        match event {
            VcsEvent::Opened(_) => self.load_branches(),
            VcsEvent::Branches(list) => {
                let signature: Vec<(String, String)> = list
                    .local
                    .iter()
                    .chain(&list.remote)
                    .map(|b| (b.name.clone(), b.last_commit.id.clone()))
                    .collect();
                if signature != self.branch_sig {
                    self.branch_sig = signature;
                    self.git.reset_log_now();
                }
                self.branches = Some(list.clone());
                if let Some(popup) = &mut self.branch_popup {
                    popup.rebuild(list);
                }
            }
            VcsEvent::BranchNameChecked { name, problem } => match &mut self.git_dialog {
                Some(GitDialog::NewBranch(d)) if d.sent == *name => {
                    d.problem = problem.clone();
                    d.checked = Some(name.clone());
                }
                Some(GitDialog::Rename(d)) if d.sent == *name => {
                    d.problem = problem.clone();
                    d.checked = Some(name.clone());
                }
                _ => {}
            },
            VcsEvent::BranchCreated(outcome) => {
                self.pending_switch = None;
                self.toast(
                    t::branch_created(&outcome.name, outcome.switched.is_some()),
                    false,
                    None,
                );
                if let Some(switched) = &outcome.switched {
                    self.after_switch(switched, false);
                }
            }
            VcsEvent::BranchRenamed(outcome) => {
                self.toast(t::branch_renamed(&outcome.old, &outcome.new), false, None);
            }
            VcsEvent::BranchDeleted(outcome) => {
                self.toast(
                    t::branch_deleted(&outcome.name),
                    false,
                    Some(ToastAction::RestoreBranch {
                        name: outcome.name.clone(),
                        tip: outcome.tip.clone(),
                    }),
                );
                if let Some((remote, branch)) = self.delete_remote_after.take() {
                    self.git_dialog =
                        Some(GitDialog::DeleteRemote(gd::DeleteRemote { remote, branch }));
                }
            }
            VcsEvent::BranchRestored { name, .. } => {
                self.toast(t::branch_restored(name), false, None);
            }
            VcsEvent::RemoteBranchDeleted(outcome) => {
                self.toast(
                    t::remote_branch_deleted(&outcome.remote, &outcome.branch),
                    false,
                    None,
                );
            }
            VcsEvent::Switched(outcome) => {
                self.pending_switch = None;
                self.after_switch(outcome, true);
            }
            VcsEvent::Fetched(outcome) => {
                self.net_progress = None;
                self.last_fetch = Some(Instant::now());
                self.toast(
                    t::fetched(outcome.updated(), outcome.pruned(), outcome.is_empty()),
                    false,
                    None,
                );
            }
            VcsEvent::BranchFastForwarded { branch, .. } => {
                self.toast(t::fast_forwarded(branch), false, None);
            }
            VcsEvent::Updated(outcome) => self.after_integration(outcome, true),
            VcsEvent::Pulled(outcome) | VcsEvent::OperationRan(outcome) => {
                self.after_integration(outcome, false);
            }
            VcsEvent::OperationSkipped(outcome) => {
                self.toast(t::skipped(outcome.finished), false, None);
            }
            VcsEvent::PushPreview(preview) => {
                if let Some(GitDialog::Push(d)) = &mut self.git_dialog {
                    let same = d.remote == preview.target.remote
                        && d.remote_branch == preview.target.remote_branch;
                    let first = d.remote.is_empty() && d.remote_branch.is_empty();
                    if first || same {
                        d.remote = preview.target.remote.clone();
                        d.remote_branch = preview.target.remote_branch.clone();
                        d.preview = Some(preview.clone());
                    }
                }
            }
            VcsEvent::Pushed(outcome) => {
                self.net_progress = None;
                self.toast(
                    t::pushed(
                        outcome.pushed,
                        &outcome.target.remote,
                        &outcome.target.remote_branch,
                        outcome.kind,
                    ),
                    false,
                    None,
                );
            }
            VcsEvent::Progress(progress) => {
                self.net_progress = Some(t::progress_text(progress));
            }
            VcsEvent::ResetPreview(preview) => {
                if let Some(GitDialog::Reset(d)) = &mut self.git_dialog {
                    if d.target == preview.target {
                        d.preview = Some(preview.clone());
                    }
                }
            }
            VcsEvent::ResetDone(outcome) => {
                self.git.invalidate_diff();
                self.toast(
                    t::reset_done(&outcome.head, outcome.stash.is_some()),
                    false,
                    None,
                );
            }
            VcsEvent::Staged { paths } => self.toast(t::staged(paths.len()), false, None),
            VcsEvent::Unstaged { paths } => self.toast(t::unstaged(paths.len()), false, None),
            VcsEvent::Moved { from, to } => self.after_move(from, to),
            VcsEvent::Discarded(outcome) => {
                if std::mem::take(&mut self.discard_delete_added) {
                    // Case « Supprimer aussi les fichiers ajoutés » : vers la corbeille.
                    self.trash_added(outcome.unstaged_added.clone());
                }
                for path in &outcome.restored {
                    if self.editors.index_of(path).is_some() {
                        julie_text::spawn_load(&self.tasks, path.clone());
                    }
                }
                self.git.invalidate_diff();
                let copy = self.last_copy.take();
                self.toast(
                    t::discarded(outcome.restored.len(), outcome.unstaged_added.len()),
                    false,
                    copy.map(ToastAction::RestoreCopy),
                );
            }
            VcsEvent::IgnoreAdded(outcome) => {
                self.tree.refresh();
                self.toast(
                    t::ignore_added(&outcome.line, outcome.already_present),
                    false,
                    None,
                );
            }
            VcsEvent::Waiting { .. } => self.toast(t::WAITING_FOR_GIT, false, None),
            _ => {}
        }
    }

    fn after_switch(&mut self, outcome: &SwitchOutcome, announce: bool) {
        self.git.invalidate_diff();
        let name = outcome
            .branch
            .clone()
            .unwrap_or_else(|| outcome.head.chars().take(7).collect());
        // Des changements locaux existaient avant la bascule : ils ont suivi (ou ont été remis).
        let mut followed = self.git_status().is_some_and(|s| !s.files.is_empty());
        if let Some(stash) = &outcome.stash {
            match &stash.restore {
                StashRestore::Restored => followed = true,
                StashRestore::Conflicts(_) | StashRestore::Pending | StashRestore::Failed(_) => {
                    self.show_git_changes();
                    self.toast(t::stash_kept(&stash.message), true, None);
                    return;
                }
                StashRestore::Kept => {}
            }
        }
        if let Some(output) = &outcome.hook_output {
            self.open_message(
                t::HOOK_FAILED_TITLE.to_owned(),
                output.clone(),
                true,
                Vec::new(),
            );
        }
        if let Some(behind) = outcome.behind_upstream.filter(|n| *n > 0) {
            self.toast(t::switched_behind(&name, behind), false, None);
        } else if announce {
            self.toast(t::switched(&name, outcome.created, followed), false, None);
        }
    }

    fn after_integration(&mut self, outcome: &OperationOutcome, update: bool) {
        self.git.invalidate_diff();
        self.git.reset_log_now();
        self.net_progress = None;
        if !outcome.finished {
            self.after_update_push = None;
            self.show_git_changes();
            self.toast(
                t::operation_stopped(outcome.kind, outcome.conflicts.len()),
                true,
                None,
            );
            return;
        }
        if let Some(stash) = &outcome.stash {
            if matches!(
                stash.restore,
                StashRestore::Conflicts(_) | StashRestore::Pending | StashRestore::Failed(_)
            ) {
                self.after_update_push = None;
                self.show_git_changes();
                self.toast(t::stash_kept(&stash.message), true, None);
                return;
            }
        }
        if update {
            if let Some(request) = self.after_update_push.take() {
                if let Some((vcs, repo)) = self.vcs_pair() {
                    vcs.push(&repo, request);
                    return;
                }
            }
        }
        let text = if outcome.up_to_date {
            t::UP_TO_DATE.to_owned()
        } else {
            t::integrated(
                outcome.kind,
                outcome.received,
                outcome.changed_files,
                outcome.fast_forward,
            )
        };
        self.toast(text, false, None);
    }

    /// Un échec d'une action v1 : dialogue explicatif plutôt qu'une ligne d'état.
    /// Rend `true` s'il l'a traité.
    pub(super) fn v1_failure(&mut self, failure: &julie_vcs::Failure) -> bool {
        self.net_progress = None;
        match (&failure.action, &failure.error) {
            (VcsAction::CheckBranchName, _) => true,
            (VcsAction::Switch | VcsAction::CreateBranch, VcsError::WouldOverwrite(conflict)) => {
                self.open_switch_overwrite(conflict.clone());
                true
            }
            (
                VcsAction::DeleteBranch,
                VcsError::BranchNotMerged {
                    branch,
                    tip,
                    commits,
                    subjects,
                },
            ) => {
                self.git_dialog = Some(GitDialog::DeleteUnmerged(gd::DeleteUnmerged {
                    name: branch.clone(),
                    tip: tip.clone(),
                    commits: *commits,
                    subjects: subjects.clone(),
                }));
                true
            }
            (_, VcsError::AuthenticationFailed { stderr }) => {
                self.after_update_push = None;
                self.open_message(
                    t::AUTH_TITLE.to_owned(),
                    format!("{}\n\n{}", t::AUTH_TEXT, stderr.trim()),
                    true,
                    vec![("terminal", t::BTN_OPEN_TERMINAL.to_owned())],
                );
                true
            }
            (_, VcsError::NoUpstream { branch }) => {
                self.open_message(
                    t::NO_UPSTREAM_TITLE.to_owned(),
                    t::no_upstream_text(branch),
                    false,
                    vec![("push_new", t::BTN_PUSH_TO_CREATE.to_owned())],
                );
                true
            }
            (_, VcsError::UpstreamGone { branch, upstream }) => {
                self.open_message(
                    t::UPSTREAM_GONE_TITLE.to_owned(),
                    t::upstream_gone_text(branch, upstream),
                    false,
                    vec![("push_new", t::BTN_PUSH_TO_RECREATE.to_owned())],
                );
                true
            }
            (_, VcsError::DetachedHead) => {
                self.toast(t::DETACHED_NOT_POSSIBLE, true, None);
                true
            }
            (VcsAction::Push, VcsError::PushRejected { rejection, output }) => {
                match rejection {
                    PushRejection::NonFastForward { known_behind } => {
                        if let Some(request) = self.last_push.clone() {
                            self.git_dialog = Some(GitDialog::PushRejected(gd::PushRejected {
                                request,
                                known_behind: *known_behind,
                            }));
                        }
                    }
                    PushRejection::StaleLease => {
                        self.open_message(
                            t::PUSH_STALE_TITLE.to_owned(),
                            format!("{}\n\n{}", t::PUSH_STALE_TEXT, output.trim()),
                            true,
                            Vec::new(),
                        );
                    }
                    PushRejection::RemoteRejected { .. } | PushRejection::Other { .. } => {
                        self.open_message(
                            t::PUSH_REFUSED_TITLE.to_owned(),
                            output.clone(),
                            true,
                            Vec::new(),
                        );
                    }
                }
                true
            }
            (_, VcsError::ProtectedBranch(branch)) => {
                self.toast(t::protected_refused(branch), true, None);
                true
            }
            (VcsAction::Move, VcsError::NotTracked(_)) => {
                // Non suivi par Git : renommage simple.
                if let Some((from, to)) = self.pending_rename.take() {
                    self.plain_rename(from, to);
                }
                true
            }
            (VcsAction::Move, VcsError::DestinationExists(_)) => {
                self.pending_rename = None;
                self.toast(t::NAME_EXISTS, true, None);
                true
            }
            (_, VcsError::IgnoredPaths(_)) => {
                self.toast(t::IGNORED_PATHS, true, None);
                true
            }
            (_, VcsError::AlreadyTracked(path)) => {
                self.toast(t::already_tracked(&path.display().to_string()), true, None);
                true
            }
            (_, VcsError::Conflicted(_)) => {
                self.toast(t::DISCARD_CONFLICTED, true, None);
                true
            }
            (_, VcsError::OperationInProgress(_)) => {
                self.toast(t::OPERATION_IN_PROGRESS, true, None);
                true
            }
            (action, error @ VcsError::GitFailed { .. }) if shows_output(*action) => {
                self.after_update_push = None;
                self.open_message(
                    t::GIT_FAILED_TITLE.to_owned(),
                    crate::gitview::failure_text(error),
                    true,
                    Vec::new(),
                );
                true
            }
            _ => false,
        }
    }

    fn open_switch_overwrite(&mut self, conflict: OverwriteConflict) {
        let root = self.project_root().unwrap_or_default();
        let files: Vec<PathBuf> = conflict.files().map(|f| relative_to(&root, f)).collect();
        let (target, can_force) = match &self.pending_switch {
            Some(Pending::Switch(target)) => (target_label(target), true),
            Some(Pending::Create(request)) => (request.name.clone(), false),
            None => (String::new(), false),
        };
        self.git_dialog = Some(GitDialog::SwitchOverwrite(gd::SwitchOverwrite {
            target,
            files,
            conflict,
            can_force,
        }));
    }

    // ----- Demandes --------------------------------------------------------------

    pub(super) fn request_switch(&mut self, target: SwitchTarget, local: LocalChanges) {
        let Some((vcs, repo)) = self.vcs_pair() else {
            return;
        };
        if let (SwitchTarget::Branch(name), Some(current)) = (&target, self.current_branch()) {
            if *name == current {
                self.toast(t::already_on(name), false, None);
                return;
            }
        }
        self.pending_switch = Some(Pending::Switch(target.clone()));
        vcs.switch_branch(
            &repo,
            SwitchRequest {
                target,
                local_changes: local,
            },
        );
    }

    pub(super) fn open_new_branch(&mut self, start: Option<String>, label: String) {
        if self.vcs_pair().is_none() {
            return;
        }
        self.branch_popup = None;
        self.git_dialog = Some(GitDialog::NewBranch(gd::NewBranch {
            name: String::new(),
            start,
            start_label: label,
            switch: true,
            problem: None,
            checked: None,
            sent: String::new(),
        }));
    }

    pub(super) fn open_rename(&mut self, old: String) {
        let upstream = self
            .branches
            .as_ref()
            .and_then(|l| l.find_local(&old))
            .and_then(|b| b.tracking.as_ref().map(|t| t.upstream.clone()));
        self.branch_popup = None;
        self.git_dialog = Some(GitDialog::Rename(gd::Rename {
            name: old.clone(),
            old,
            upstream,
            problem: None,
            checked: None,
            sent: String::new(),
        }));
    }

    pub(super) fn open_push_dialog(&mut self, branch: Option<String>) {
        let Some(branch) = branch.or_else(|| self.current_branch()) else {
            self.toast(t::DETACHED_NOT_POSSIBLE, true, None);
            return;
        };
        self.branch_popup = None;
        self.git_dialog = Some(GitDialog::Push(gd::Push {
            branch,
            remote: String::new(),
            remote_branch: String::new(),
            preview: None,
            dirty: true,
            follow_tags: false,
            confirm_force: false,
            selected: 0,
        }));
    }

    pub(super) fn do_update(&mut self, method: IntegrationMethod) {
        if let Some((vcs, repo)) = self.vcs_pair() {
            vcs.update_project(
                &repo,
                UpdateRequest {
                    method,
                    local_changes: LocalChanges::Stash,
                },
            );
        }
    }

    pub(super) fn start_update(&mut self) {
        let Some(branch) = self.current_branch() else {
            self.toast(t::DETACHED_NOT_POSSIBLE, true, None);
            return;
        };
        match self.update_method {
            Some(method) => self.do_update(method),
            None => {
                let upstream = self
                    .git_status()
                    .and_then(|s| s.upstream.as_ref().map(|u| u.name.clone()));
                self.branch_popup = None;
                self.git_dialog = Some(GitDialog::UpdateMethod(gd::UpdateMethod {
                    branch,
                    upstream,
                    remember: false,
                }));
            }
        }
    }

    pub(super) fn open_reset(&mut self, target: String, short: String, subject: String) {
        let Some((vcs, repo)) = self.vcs_pair() else {
            return;
        };
        vcs.load_reset_preview(&repo, target.clone());
        self.git_dialog = Some(GitDialog::Reset(gd::Reset {
            target,
            short,
            subject,
            mode: ResetUiMode::Mixed,
            preview: None,
            stash_first: true,
            confirm_hard: false,
            branch: self.current_branch().unwrap_or_default(),
        }));
    }

    // ----- Dialogues ------------------------------------------------------------

    /// Vérifie les noms à la frappe, relit l'aperçu du push quand la cible change.
    pub(super) fn poll_dialogs(&mut self) {
        let Some((vcs, repo)) = self.vcs_pair() else {
            return;
        };
        match &mut self.git_dialog {
            Some(GitDialog::NewBranch(d)) if d.name != d.sent => {
                d.sent.clone_from(&d.name);
                d.problem = None;
                d.checked = None;
                if !d.name.trim().is_empty() {
                    vcs.check_branch_name(&repo, d.name.clone(), None);
                }
            }
            Some(GitDialog::Rename(d)) if d.name != d.sent => {
                d.sent.clone_from(&d.name);
                d.problem = None;
                d.checked = None;
                if !d.name.trim().is_empty() && d.name != d.old {
                    vcs.check_branch_name(&repo, d.name.clone(), Some(d.old.clone()));
                }
            }
            Some(GitDialog::Push(d)) if d.dirty => {
                d.dirty = false;
                d.preview = None;
                d.selected = 0;
                let target = (!d.remote.trim().is_empty() && !d.remote_branch.trim().is_empty())
                    .then(|| PushTarget {
                        branch: d.branch.clone(),
                        remote: d.remote.trim().to_owned(),
                        remote_branch: d.remote_branch.trim().to_owned(),
                    });
                vcs.load_push_preview(&repo, Some(d.branch.clone()), target);
            }
            _ => {}
        }
    }

    pub(super) fn draw_git_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        self.poll_dialogs();
        let Some(dialog) = &mut self.git_dialog else {
            return;
        };
        if let Some(key) = dialog.show(ctx, p) {
            self.dialog_choose(key);
        }
    }

    /// Un bouton de dialogue a été pressé.
    pub(super) fn dialog_choose(&mut self, key: &str) {
        let Some(mut dialog) = self.git_dialog.take() else {
            return;
        };
        let enabled = dialog
            .buttons()
            .iter()
            .find(|b| b.key == key)
            .is_none_or(|b| b.enabled);
        if !enabled {
            self.git_dialog = Some(dialog);
            return;
        }
        if matches!(key, "cancel" | "close") {
            self.focus_editor();
            return;
        }
        let pair = self.vcs_pair();
        match (&mut dialog, key) {
            (GitDialog::NewBranch(d), "use_suggestion") => {
                if let Some(julie_vcs::BranchNameProblem::Invalid {
                    suggestion: Some(s),
                }) = &d.problem
                {
                    d.name = s.clone();
                }
                self.git_dialog = Some(dialog);
            }
            (GitDialog::Rename(d), "use_suggestion") => {
                if let Some(julie_vcs::BranchNameProblem::Invalid {
                    suggestion: Some(s),
                }) = &d.problem
                {
                    d.name = s.clone();
                }
                self.git_dialog = Some(dialog);
            }
            (GitDialog::NewBranch(d), "create") => {
                if let Some((vcs, repo)) = pair {
                    let request = CreateBranchRequest {
                        name: d.name.trim().to_owned(),
                        start: d.start.clone(),
                        switch: d.switch,
                        local_changes: LocalChanges::Keep,
                    };
                    self.pending_switch = Some(Pending::Create(request.clone()));
                    vcs.create_branch(&repo, request);
                }
            }
            (GitDialog::Rename(d), "rename") => {
                if let Some((vcs, repo)) = pair {
                    vcs.rename_branch(&repo, d.old.clone(), d.name.trim().to_owned());
                }
            }
            (GitDialog::SwitchOverwrite(d), "stash") => {
                if let Some((vcs, repo)) = pair {
                    match self.pending_switch.take() {
                        Some(Pending::Switch(target)) => {
                            self.pending_switch = Some(Pending::Switch(target.clone()));
                            vcs.switch_branch(
                                &repo,
                                SwitchRequest {
                                    target,
                                    local_changes: LocalChanges::Stash,
                                },
                            );
                        }
                        Some(Pending::Create(mut request)) => {
                            request.local_changes = LocalChanges::Stash;
                            self.pending_switch = Some(Pending::Create(request.clone()));
                            vcs.create_branch(&repo, request);
                        }
                        None => {}
                    }
                }
                let _ = d;
            }
            (GitDialog::SwitchOverwrite(d), "force") => {
                self.git_dialog = Some(GitDialog::ForceSwitch(gd::ForceSwitch {
                    target: d.target.clone(),
                    files: d.files.clone(),
                    conflict: d.conflict.clone(),
                }));
            }
            (GitDialog::ForceSwitch(d), "force_confirm") => {
                if let (Some(Pending::Switch(target)), Some(root)) =
                    (&self.pending_switch, self.project_root())
                {
                    let files: Vec<PathBuf> = d
                        .conflict
                        .files()
                        .map(|f| {
                            if f.is_relative() {
                                root.join(f)
                            } else {
                                f.clone()
                            }
                        })
                        .collect();
                    project::spawn_safety_copy(
                        root,
                        files,
                        AfterCopy::ForceSwitch {
                            target: target.clone(),
                            conflict: d.conflict.clone(),
                        },
                        self.channel.clone(),
                    );
                }
            }
            (GitDialog::UpdateMethod(d), "merge" | "rebase") => {
                let method = if key == "merge" {
                    IntegrationMethod::Merge
                } else {
                    IntegrationMethod::Rebase
                };
                if d.remember {
                    // « Ne plus demander » : réglage [vcs] update_method de ce projet.
                    self.update_method = Some(method);
                    let value = if method == IntegrationMethod::Merge {
                        "fusion"
                    } else {
                        "rebase"
                    };
                    self.write_setting(
                        julie_core::settings::Scope::Project,
                        &["vcs", "update_method"],
                        Some(julie_core::settings::SettingValue::Text(value.to_owned())),
                    );
                    self.toast(t::SETTINGS_REMEMBER_SAVED_TOAST, false, None);
                }
                self.do_update(method);
            }
            (GitDialog::Push(d), "push") => {
                if let (Some((vcs, repo)), Some(preview)) = (pair, &d.preview) {
                    let request = PushRequest {
                        target: PushTarget {
                            branch: d.branch.clone(),
                            remote: d.remote.trim().to_owned(),
                            remote_branch: d.remote_branch.trim().to_owned(),
                        },
                        set_upstream: preview.sets_upstream,
                        follow_tags: d.follow_tags,
                        force: PushForce::No,
                    };
                    self.last_push = Some(request.clone());
                    vcs.push(&repo, request);
                }
            }
            (GitDialog::Push(d), "force") => {
                d.confirm_force = true;
                self.git_dialog = Some(dialog);
            }
            (GitDialog::Push(d), "back") => {
                d.confirm_force = false;
                self.git_dialog = Some(dialog);
            }
            (GitDialog::Push(d), "force_confirm") => {
                if let (Some((vcs, repo)), Some(preview)) = (pair, &d.preview) {
                    if !preview.protected {
                        let request = PushRequest {
                            target: PushTarget {
                                branch: d.branch.clone(),
                                remote: d.remote.trim().to_owned(),
                                remote_branch: d.remote_branch.trim().to_owned(),
                            },
                            set_upstream: preview.sets_upstream,
                            follow_tags: d.follow_tags,
                            force: PushForce::WithLease { expected: None },
                        };
                        self.last_push = Some(request.clone());
                        vcs.push(&repo, request);
                    }
                }
            }
            (GitDialog::PushRejected(d), "merge_push" | "rebase_push") => {
                self.after_update_push = Some(d.request.clone());
                self.do_update(if key == "merge_push" {
                    IntegrationMethod::Merge
                } else {
                    IntegrationMethod::Rebase
                });
            }
            (GitDialog::DeleteBranch(d), "delete") => {
                if let Some((vcs, repo)) = pair {
                    if d.also_remote {
                        if let Some(upstream) = &d.upstream {
                            if let Some((remote, branch)) = upstream.split_once('/') {
                                self.delete_remote_after =
                                    Some((remote.to_owned(), branch.to_owned()));
                            }
                        }
                    }
                    vcs.delete_branch(&repo, d.name.clone());
                }
            }
            (GitDialog::DeleteUnmerged(d), "force_delete") => {
                if let Some((vcs, repo)) = pair {
                    vcs.force_delete_branch(&repo, d.name.clone());
                }
            }
            (GitDialog::DeleteRemote(d), "delete_remote") => {
                if let Some((vcs, repo)) = pair {
                    vcs.delete_remote_branch(&repo, d.remote.clone(), d.branch.clone());
                }
            }
            (GitDialog::Reset(d), "reset") => {
                let local = d
                    .preview
                    .as_ref()
                    .map(|p| p.local_changes.clone())
                    .unwrap_or_default();
                match d.mode {
                    ResetUiMode::Soft | ResetUiMode::Mixed => {
                        if let Some((vcs, repo)) = pair {
                            let mode = if d.mode == ResetUiMode::Soft {
                                ResetMode::Soft
                            } else {
                                ResetMode::Mixed
                            };
                            vcs.reset(&repo, d.target.clone(), mode);
                        }
                    }
                    ResetUiMode::Hard if !local.is_empty() && !d.confirm_hard => {
                        d.confirm_hard = true;
                        self.git_dialog = Some(dialog);
                    }
                    ResetUiMode::Hard => {
                        let stash_first = d.stash_first && !local.is_empty();
                        if local.is_empty() || stash_first {
                            if let Some((vcs, repo)) = pair {
                                vcs.reset_hard(&repo, d.target.clone(), stash_first);
                            }
                        } else if let Some(root) = self.project_root() {
                            let files = local
                                .iter()
                                .map(|f| {
                                    if f.is_relative() {
                                        root.join(f)
                                    } else {
                                        f.clone()
                                    }
                                })
                                .collect();
                            project::spawn_safety_copy(
                                root,
                                files,
                                AfterCopy::ResetHard {
                                    target: d.target.clone(),
                                    stash_first: false,
                                },
                                self.channel.clone(),
                            );
                        }
                    }
                }
            }
            (GitDialog::Discard(d), "discard") => {
                self.discard_delete_added = d.delete_added;
                if let Some(root) = self.project_root() {
                    project::spawn_safety_copy(
                        root,
                        d.paths.clone(),
                        AfterCopy::Discard {
                            paths: d.paths.clone(),
                        },
                        self.channel.clone(),
                    );
                }
            }
            (GitDialog::CheckoutCommit(d), "checkout") => {
                self.request_switch(SwitchTarget::Detach(d.id.clone()), LocalChanges::Keep);
            }
            (GitDialog::Message(_), "terminal") => {
                self.tools.bottom = BottomTool::Terminal;
                self.tools.bottom_open = true;
                if let Some(terminal) = &mut self.terminal {
                    terminal.focus();
                }
            }
            (GitDialog::Message(_), "push_new") => self.open_push_dialog(None),
            _ => {}
        }
    }

    /// La copie de sécurité est faite : l'opération destructive peut partir.
    pub(super) fn on_safety_copied(
        &mut self,
        then: AfterCopy,
        result: Result<Option<String>, String>,
    ) {
        let id = match result {
            Ok(id) => id,
            Err(error) => {
                self.toast(t::safety_failed(&error), true, None);
                return;
            }
        };
        self.last_copy = id;
        let Some((vcs, repo)) = self.vcs_pair() else {
            return;
        };
        match then {
            AfterCopy::Discard { paths } => {
                vcs.discard_changes(&repo, paths);
            }
            AfterCopy::ForceSwitch { target, conflict } => {
                self.pending_switch = None;
                vcs.switch_discarding_changes(&repo, target, conflict);
            }
            AfterCopy::ResetHard {
                target,
                stash_first,
            } => {
                vcs.reset_hard(&repo, target, stash_first);
            }
        }
    }

    /// Le bouton de la notification (« Restaurer »…).
    pub(super) fn toast_action(&mut self) {
        let Some(action) = self.message.as_mut().and_then(|m| m.action.take()) else {
            return;
        };
        match action {
            ToastAction::RestoreBranch { name, tip } => {
                if let Some((vcs, repo)) = self.vcs_pair() {
                    vcs.restore_branch(&repo, name, tip);
                }
            }
            ToastAction::RestoreCopy(id) => {
                if let Some(root) = self.project_root() {
                    project::spawn_restore_copy(root, id, self.channel.clone());
                }
            }
            ToastAction::OpenFile(path, line) => {
                let goto = line.map(|line| Goto {
                    position: TextPosition::new(line.saturating_sub(1), 0),
                    end: None,
                });
                self.open_file(path, goto);
            }
        }
    }

    // ----- Mode scénario ---------------------------------------------------------

    pub(super) fn v1_state(&self) -> serde_json::Value {
        let branch_json = |b: &julie_vcs::Branch| {
            serde_json::json!({
                "name": b.name,
                "current": b.current,
                "upstream": b.tracking.as_ref().map(|t| t.upstream.clone()),
                "ahead": b.tracking.as_ref().map(|t| t.ahead),
                "behind": b.tracking.as_ref().map(|t| t.behind),
                "gone": b.tracking.as_ref().map(|t| t.gone),
                "subject": b.last_commit.subject,
            })
        };
        let widget = self.widget_info().map(|w| {
            serde_json::json!({
                "text": w.text, "ahead": w.ahead, "behind": w.behind,
                "kind": format!("{:?}", w.kind),
            })
        });
        serde_json::json!({
            "branches": self.branches.as_ref().map(|l| serde_json::json!({
                "current": l.current,
                "detached_at": l.detached_at,
                "unborn": l.unborn,
                "local": l.local.iter().map(branch_json).collect::<Vec<_>>(),
                "remote": l.remote.iter().map(branch_json).collect::<Vec<_>>(),
                "recent": l.recent,
                "remotes": l.remotes,
            })),
            "branch_widget": widget,
            "branch_popup": self.branch_popup.as_ref().map(|p| p.state(&self.registry)),
            "context_menu": self.ctx_menu.as_ref().map(ContextMenu::state),
            "git_dialog": self.git_dialog.as_ref().map(GitDialog::state),
            "toast": self.message.as_ref().map(|m| serde_json::json!({
                "text": m.text, "error": m.error,
                "action": m.action.as_ref().map(|a| match a {
                    ToastAction::RestoreBranch { name, .. } => format!("restore_branch:{name}"),
                    ToastAction::RestoreCopy(id) => format!("restore_copy:{id}"),
                    ToastAction::OpenFile(path, _) => format!("open_file:{}", path.display()),
                }),
            })),
            "update_method_session": self.update_method.map(|m| format!("{m:?}")),
            "safety_copy": self.last_copy,
            "net_progress": self.net_progress,
            "settings": self.settings_state(),
        })
    }

    /// Les commandes `ui` des branches, menus, dialogues. `None` : commande inconnue ici.
    pub(super) fn ui_command_v1(&mut self, command: &str, arg: &str) -> Option<Result<(), String>> {
        let result = match command {
            "branches.open" => {
                if self.branch_popup.is_none() {
                    self.toggle_branch_popup();
                }
                Ok(())
            }
            "branches.close" => {
                self.branch_popup = None;
                self.ctx_menu = None;
                Ok(())
            }
            "branches.query" => match &mut self.branch_popup {
                Some(popup) => {
                    popup.query = arg.to_owned();
                    Ok(())
                }
                None => Err("la fenêtre des branches n'est pas ouverte".to_owned()),
            },
            "branches.select" => match &mut self.branch_popup {
                Some(popup) => {
                    if popup.select_branch(arg) {
                        Ok(())
                    } else {
                        Err(format!("branche absente de la fenêtre : {arg}"))
                    }
                }
                None => Err("la fenêtre des branches n'est pas ouverte".to_owned()),
            },
            "branches.menu" => {
                let branch = match &mut self.branch_popup {
                    Some(popup) => {
                        if !arg.is_empty() && !popup.select_branch(arg) {
                            return Some(Err(format!("branche absente de la fenêtre : {arg}")));
                        }
                        popup.selected_branch()
                    }
                    None => return Some(Err("la fenêtre des branches n'est pas ouverte".into())),
                };
                match branch {
                    Some(branch) => {
                        let at = self.branch_widget.map_or(egui::pos2(900.0, 120.0), |r| {
                            egui::pos2(r.right() - 20.0, r.bottom() + 120.0)
                        });
                        self.open_menu(Target::Branch(branch), at, false);
                        Ok(())
                    }
                    None => Err("la ligne choisie n'est pas une branche".to_owned()),
                }
            }
            "menu.open" => self.scenario_open_menu(arg),
            "menu.expand" => match &mut self.ctx_menu {
                Some(menu) => {
                    if menu.expand(arg) {
                        Ok(())
                    } else {
                        Err(format!("pas de sous-menu « {arg} »"))
                    }
                }
                None => Err("aucun menu ouvert".to_owned()),
            },
            "menu.choose" => self.scenario_choose(arg),
            "menu.close" => {
                self.ctx_menu = None;
                Ok(())
            }
            "dialog.set" => {
                let (name, value) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                match &mut self.git_dialog {
                    Some(dialog) => dialog.set_field(name, value.trim()),
                    None => Err("aucun dialogue Git ouvert".to_owned()),
                }
            }
            "dialog.choose" => match &self.git_dialog {
                Some(dialog) => match dialog.buttons().iter().find(|b| b.key == arg) {
                    Some(button) if button.enabled => {
                        self.dialog_choose(arg);
                        Ok(())
                    }
                    Some(button) => Err(format!(
                        "bouton « {arg} » grisé : {}",
                        button.tip.clone().unwrap_or_default()
                    )),
                    None => Err(format!("pas de bouton « {arg} » dans ce dialogue")),
                },
                None => Err("aucun dialogue Git ouvert".to_owned()),
            },
            "toast.action" => {
                if self.message.as_ref().is_some_and(|m| m.action.is_some()) {
                    self.toast_action();
                    Ok(())
                } else {
                    Err("la notification n'a pas de bouton".to_owned())
                }
            }
            "op.continue" => match self.git.continue_intent() {
                Ok(intent) => {
                    self.run_git_intents(vec![intent]);
                    Ok(())
                }
                Err(reason) => Err(reason),
            },
            "op.skip" => {
                if self.git.ask_skip() {
                    Ok(())
                } else {
                    Err("pas de commit à passer (pas de rebase en cours)".to_owned())
                }
            }
            "net.cancel" => {
                let task = self
                    .tracker
                    .running_of(crate::status::cancellable)
                    .into_iter()
                    .next();
                match task {
                    Some(id) => {
                        self.tasks.cancel(id);
                        Ok(())
                    }
                    None => Err("aucune opération réseau en cours".to_owned()),
                }
            }
            "git.file-history" => {
                let Some(root) = self.project_root() else {
                    return Some(Err("pas de projet".into()));
                };
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git
                    .show_file_history(root.join(arg.replace('/', "\\")));
                Ok(())
            }
            "git.log-filter" => {
                let (name, value) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                let mut filters = self.git.log.filters.clone();
                match name {
                    "text" => filters.text = value.trim().to_owned(),
                    "author" => filters.author = value.trim().to_owned(),
                    "since" => filters.since = value.trim().to_owned(),
                    "until" => filters.until = value.trim().to_owned(),
                    "branch" => {
                        filters.branch = (!value.trim().is_empty()).then(|| value.trim().to_owned())
                    }
                    "clear" => filters = crate::logview::Filters::default(),
                    other => {
                        return Some(Err(format!(
                        "filtre inconnu : « {other} » (text, author, since, until, branch, clear)"
                    )))
                    }
                }
                self.tools.bottom = BottomTool::Git;
                self.tools.bottom_open = true;
                self.git.tab = GitTab::Log;
                self.git.log.set_filters(filters);
                Ok(())
            }
            _ => return None,
        };
        Some(result)
    }
}

/// Chemin affiché : relatif au projet quand c'est possible.
fn relative_to(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| path.to_path_buf())
}

fn target_label(target: &SwitchTarget) -> String {
    match target {
        SwitchTarget::Branch(name) | SwitchTarget::Remote(name) => name.clone(),
        SwitchTarget::Detach(rev) => rev.chars().take(7).collect(),
    }
}
