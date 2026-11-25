//! Dialogues des branches et des opérations Git : création et renommage d'une
//! branche (nom vérifié en direct), bascule avec changements locaux, mise à jour
//! du projet, push, suppressions, réinitialisation, annulation des modifications,
//! sorties de Git.
//!
//! Ce module ne contient **aucune règle Git** : il garde l'état d'un dialogue,
//! le dessine, et rend la clé du bouton pressé. `app/gitflow.rs` traduit la
//! réponse en appels à `julie-vcs`.

use std::path::PathBuf;

use eframe::egui::{self, vec2, Context, Key, Modifiers, RichText, TextEdit, Ui};
use julie_vcs::{
    BranchNameProblem, CommitSummary, OverwriteConflict, PushPreview, PushRequest, ResetPreview,
};

use crate::kit;
use crate::texts as t;
use crate::theme::Palette;

/// Un bouton du pied de dialogue.
#[derive(Debug, Clone)]
pub struct Btn {
    pub key: &'static str,
    pub label: String,
    pub primary: bool,
    pub danger: bool,
    pub enabled: bool,
    pub tip: Option<String>,
}

impl Btn {
    fn new(key: &'static str, label: impl Into<String>) -> Self {
        Self {
            key,
            label: label.into(),
            primary: false,
            danger: false,
            enabled: true,
            tip: None,
        }
    }

    fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    fn when(mut self, enabled: bool, why: &str) -> Self {
        self.enabled = enabled;
        if !enabled && !why.is_empty() {
            self.tip = Some(why.to_owned());
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetUiMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetUiMode {
    pub fn key(self) -> &'static str {
        match self {
            ResetUiMode::Soft => "soft",
            ResetUiMode::Mixed => "mixed",
            ResetUiMode::Hard => "hard",
        }
    }
}

pub struct NewBranch {
    pub name: String,
    /// Point de départ (`None` : HEAD) et son libellé.
    pub start: Option<String>,
    pub start_label: String,
    pub switch: bool,
    pub problem: Option<BranchNameProblem>,
    /// Le nom pour lequel `problem` a été calculé.
    pub checked: Option<String>,
    /// Le dernier nom envoyé à la vérification.
    pub sent: String,
}

pub struct Rename {
    pub old: String,
    pub name: String,
    pub upstream: Option<String>,
    pub problem: Option<BranchNameProblem>,
    pub checked: Option<String>,
    pub sent: String,
}

pub struct SwitchOverwrite {
    pub target: String,
    pub files: Vec<PathBuf>,
    pub conflict: OverwriteConflict,
    /// « Forcer la bascule » n'existe que pour une bascule (pas pour une création).
    pub can_force: bool,
    /// Ce qui est refusé, dit pour le titre : « la bascule vers X », « la fusion de X »…
    pub what: String,
    /// Le verbe du bouton « Mettre de côté et … » (« basculer », « fusionner »…).
    pub verb: String,
    /// C'est une bascule (les textes historiques de la bascule s'appliquent).
    pub is_switch: bool,
}

/// Quitter une HEAD détachée qui porte des commits sur aucune branche.
pub struct DetachedLeft {
    pub target: String,
    pub head: String,
    pub count: u32,
    pub subjects: Vec<String>,
    /// « Basculer quand même » n'existe que pour une bascule.
    pub can_anyway: bool,
}

/// « Choisir l'amont… » : la liste des branches distantes.
pub struct SetUpstream {
    pub branch: String,
    pub choices: Vec<String>,
    pub selected: usize,
}

/// « Voir le diff avec l'arbre de travail » : les fichiers qui diffèrent.
pub struct DiffWorktree {
    pub branch: String,
    /// `None` : en cours de lecture. (statut, chemin relatif)
    pub files: Option<Vec<(String, PathBuf)>>,
    pub selected: usize,
    pub error: Option<String>,
}

pub struct ForceSwitch {
    pub target: String,
    pub files: Vec<PathBuf>,
    pub conflict: OverwriteConflict,
}

pub struct UpdateMethod {
    pub branch: String,
    pub upstream: Option<String>,
    pub remember: bool,
}

pub struct Push {
    pub branch: String,
    pub remote: String,
    pub remote_branch: String,
    pub preview: Option<PushPreview>,
    /// Cible modifiée à la main : l'aperçu est à refaire.
    pub dirty: bool,
    pub follow_tags: bool,
    pub confirm_force: bool,
    pub selected: usize,
}

pub struct PushRejected {
    pub request: PushRequest,
    pub known_behind: u32,
}

pub struct DeleteBranch {
    pub name: String,
    pub upstream: Option<String>,
    pub also_remote: bool,
}

pub struct DeleteUnmerged {
    pub name: String,
    pub tip: String,
    pub commits: u32,
    pub subjects: Vec<String>,
}

pub struct DeleteRemote {
    pub remote: String,
    pub branch: String,
}

pub struct Reset {
    pub target: String,
    pub short: String,
    pub subject: String,
    pub mode: ResetUiMode,
    pub preview: Option<ResetPreview>,
    pub stash_first: bool,
    pub confirm_hard: bool,
    pub branch: String,
}

/// Annuler les modifications : ce qui arrivera à chaque fichier.
pub struct Discard {
    pub paths: Vec<PathBuf>,
    /// (nom affiché, ce qui lui arrive)
    pub items: Vec<(String, String)>,
    /// Fichiers ajoutés : retirés de l'index, ils restent sur le disque.
    pub added: usize,
    pub unsaved: Vec<String>,
    /// « Supprimer aussi les fichiers ajoutés (vers la corbeille) » : décochée par défaut.
    pub delete_added: bool,
}

pub struct CheckoutCommit {
    pub id: String,
    pub short: String,
    pub subject: String,
}

pub struct Message {
    pub title: String,
    pub text: String,
    pub error: bool,
    /// Boutons en plus de « Fermer ».
    pub extra: Vec<(&'static str, String)>,
}

pub enum Dialog {
    NewBranch(NewBranch),
    Rename(Rename),
    SwitchOverwrite(SwitchOverwrite),
    DetachedLeft(DetachedLeft),
    SetUpstream(SetUpstream),
    DiffWorktree(DiffWorktree),
    ForceSwitch(ForceSwitch),
    UpdateMethod(UpdateMethod),
    Push(Push),
    PushRejected(PushRejected),
    DeleteBranch(DeleteBranch),
    DeleteUnmerged(DeleteUnmerged),
    DeleteRemote(DeleteRemote),
    Reset(Reset),
    Discard(Discard),
    CheckoutCommit(CheckoutCommit),
    Message(Message),
}

fn problem_text(problem: &BranchNameProblem) -> String {
    t::branch_name_problem(problem)
}

impl Dialog {
    pub fn kind(&self) -> &'static str {
        match self {
            Dialog::NewBranch(_) => "new_branch",
            Dialog::Rename(_) => "rename",
            Dialog::SwitchOverwrite(_) => "switch_overwrite",
            Dialog::DetachedLeft(_) => "detached_left",
            Dialog::SetUpstream(_) => "set_upstream",
            Dialog::DiffWorktree(_) => "diff_worktree",
            Dialog::ForceSwitch(_) => "force_switch",
            Dialog::UpdateMethod(_) => "update_method",
            Dialog::Push(_) => "push",
            Dialog::PushRejected(_) => "push_rejected",
            Dialog::DeleteBranch(_) => "delete_branch",
            Dialog::DeleteUnmerged(_) => "delete_unmerged",
            Dialog::DeleteRemote(_) => "delete_remote",
            Dialog::Reset(_) => "reset",
            Dialog::Discard(_) => "discard",
            Dialog::CheckoutCommit(_) => "checkout_commit",
            Dialog::Message(_) => "message",
        }
    }

    fn title(&self) -> String {
        match self {
            Dialog::NewBranch(_) => t::DLG_NEW_BRANCH.to_owned(),
            Dialog::Rename(d) => t::dlg_rename(&d.old),
            Dialog::SwitchOverwrite(d) => t::dlg_overwrite_for(&d.what),
            Dialog::DetachedLeft(d) => t::dlg_detached_left(d.count),
            Dialog::SetUpstream(d) => t::dlg_set_upstream(&d.branch),
            Dialog::DiffWorktree(d) => t::dlg_diff_worktree(&d.branch),
            Dialog::ForceSwitch(_) => t::DLG_FORCE_SWITCH.to_owned(),
            Dialog::UpdateMethod(_) => t::DLG_UPDATE_METHOD.to_owned(),
            Dialog::Push(d) => t::dlg_push(&d.branch),
            Dialog::PushRejected(_) => t::DLG_PUSH_REJECTED.to_owned(),
            Dialog::DeleteBranch(d) => t::dlg_delete_branch(&d.name),
            Dialog::DeleteUnmerged(d) => t::dlg_delete_unmerged(&d.name),
            Dialog::DeleteRemote(d) => t::dlg_delete_remote(&d.remote, &d.branch),
            Dialog::Reset(d) => t::dlg_reset(&d.branch, &d.short),
            Dialog::Discard(d) => t::dlg_discard(d.items.len()),
            Dialog::CheckoutCommit(d) => t::dlg_checkout_commit(&d.short),
            Dialog::Message(d) => d.title.clone(),
        }
    }

    fn width(&self) -> f32 {
        match self {
            Dialog::Push(_) => 680.0,
            Dialog::Message(_)
            | Dialog::Reset(_)
            | Dialog::DeleteUnmerged(_)
            | Dialog::DetachedLeft(_)
            | Dialog::DiffWorktree(_) => 560.0,
            _ => 470.0,
        }
    }

    pub fn buttons(&self) -> Vec<Btn> {
        let cancel = Btn::new("cancel", t::DIALOG_CANCEL);
        match self {
            Dialog::NewBranch(d) => {
                let valid = !d.name.trim().is_empty()
                    && d.problem.is_none()
                    && d.checked.as_deref() == Some(d.name.as_str());
                vec![
                    Btn::new("create", t::BTN_CREATE)
                        .primary()
                        .when(valid, t::TIP_NAME_NOT_VALID),
                    cancel,
                ]
            }
            Dialog::Rename(d) => {
                let valid = !d.name.trim().is_empty()
                    && d.name != d.old
                    && d.problem.is_none()
                    && d.checked.as_deref() == Some(d.name.as_str());
                vec![
                    Btn::new("rename", t::BTN_RENAME)
                        .primary()
                        .when(valid, t::TIP_NAME_NOT_VALID),
                    cancel,
                ]
            }
            Dialog::SwitchOverwrite(d) => {
                let label = if d.is_switch {
                    t::BTN_STASH_AND_SWITCH.to_owned()
                } else {
                    t::stash_and(&d.verb)
                };
                let mut buttons = vec![Btn::new("stash", label).primary()];
                if d.can_force {
                    buttons.push(Btn::new("force", t::BTN_FORCE_SWITCH).danger());
                }
                buttons.push(cancel);
                buttons
            }
            Dialog::DetachedLeft(d) => {
                let mut buttons =
                    vec![Btn::new("create_first", t::BTN_CREATE_BRANCH_FIRST).primary()];
                if d.can_anyway {
                    buttons.push(Btn::new("anyway", t::BTN_SWITCH_ANYWAY).danger());
                }
                buttons.push(cancel);
                buttons
            }
            Dialog::SetUpstream(d) => vec![
                Btn::new("set_upstream", t::BTN_SET_UPSTREAM)
                    .primary()
                    .when(!d.choices.is_empty(), t::UPSTREAM_PICK_NONE),
                cancel,
            ],
            Dialog::DiffWorktree(d) => vec![
                Btn::new("open_diff", t::BTN_OPEN_DIFF)
                    .primary()
                    .when(d.files.as_ref().is_some_and(|f| !f.is_empty()), ""),
                Btn::new("close", t::CLOSE),
            ],
            Dialog::ForceSwitch(_) => vec![
                cancel.primary(),
                Btn::new("force_confirm", t::BTN_FORCE_AND_DISCARD).danger(),
            ],
            Dialog::UpdateMethod(_) => vec![
                Btn::new("merge", t::BTN_METHOD_MERGE).primary(),
                Btn::new("rebase", t::BTN_METHOD_REBASE),
                cancel,
            ],
            Dialog::Push(d) => {
                if d.confirm_force {
                    return vec![
                        Btn::new("force_confirm", t::BTN_FORCE_LEASE).danger(),
                        Btn::new("back", t::BTN_BACK).primary(),
                    ];
                }
                let nothing = d.preview.as_ref().is_none_or(|p| p.outgoing.is_empty());
                let protected = d.preview.as_ref().is_some_and(|p| p.protected);
                vec![
                    Btn::new("push", t::BTN_PUSH).primary().when(
                        d.preview.is_some() && !nothing,
                        if d.preview.is_some() {
                            t::TIP_NOTHING_TO_PUSH
                        } else {
                            ""
                        },
                    ),
                    Btn::new("force", t::BTN_FORCE_LEASE_ASK).when(
                        d.preview.is_some() && !protected,
                        if protected {
                            t::TIP_PROTECTED_BRANCH
                        } else {
                            ""
                        },
                    ),
                    cancel,
                ]
            }
            Dialog::PushRejected(_) => vec![
                Btn::new("merge_push", t::BTN_MERGE_THEN_PUSH).primary(),
                Btn::new("rebase_push", t::BTN_REBASE_THEN_PUSH),
                cancel,
            ],
            Dialog::DeleteBranch(_) => vec![Btn::new("delete", t::BTN_DELETE).primary(), cancel],
            Dialog::DeleteUnmerged(_) => vec![
                cancel.primary(),
                Btn::new("force_delete", t::BTN_DELETE_ANYWAY).danger(),
            ],
            Dialog::DeleteRemote(_) => vec![
                cancel.primary(),
                Btn::new("delete_remote", t::BTN_DELETE_REMOTE).danger(),
            ],
            Dialog::Reset(d) => {
                let hard = d.mode == ResetUiMode::Hard;
                let ready = d.preview.is_some();
                let label = if hard && !d.confirm_hard && has_local(d) {
                    t::BTN_CONTINUE
                } else {
                    t::BTN_RESET
                };
                let mut button = Btn::new("reset", label).when(ready, "");
                if hard {
                    button = button.danger();
                } else {
                    button = button.primary();
                }
                vec![button, cancel]
            }
            Dialog::Discard(_) => vec![Btn::new("discard", t::BTN_DISCARD).primary(), cancel],
            Dialog::CheckoutCommit(_) => {
                vec![Btn::new("checkout", t::BTN_CHECKOUT).primary(), cancel]
            }
            Dialog::Message(d) => {
                let mut buttons: Vec<Btn> = d
                    .extra
                    .iter()
                    .enumerate()
                    .map(|(i, (key, label))| {
                        let b = Btn::new(key, label.clone());
                        if i == 0 {
                            b.primary()
                        } else {
                            b
                        }
                    })
                    .collect();
                buttons.push(Btn::new("close", t::CLOSE));
                buttons
            }
        }
    }

    /// Règle un champ par son nom (mode scénario). Rend `Err` si le dialogue n'a pas ce champ.
    pub fn set_field(&mut self, name: &str, value: &str) -> Result<(), String> {
        let flag = || match value {
            "on" | "oui" | "true" => Ok(true),
            "off" | "non" | "false" => Ok(false),
            other => Err(format!("`on` ou `off` attendu : « {other} »")),
        };
        match (self, name) {
            (Dialog::NewBranch(d), "name") => d.name = value.to_owned(),
            (Dialog::NewBranch(d), "switch") => d.switch = flag()?,
            (Dialog::Rename(d), "name") => d.name = value.to_owned(),
            (Dialog::UpdateMethod(d), "remember") => d.remember = flag()?,
            (Dialog::Push(d), "remote") => {
                d.remote = value.to_owned();
                d.dirty = true;
            }
            (Dialog::Push(d), "branch") => {
                d.remote_branch = value.to_owned();
                d.dirty = true;
            }
            (Dialog::Push(d), "follow_tags") => d.follow_tags = flag()?,
            (Dialog::Push(d), "commit") => {
                d.selected = value.parse::<usize>().map_err(|_| "numéro attendu")?
            }
            (Dialog::SetUpstream(d), "upstream") => {
                d.selected = d
                    .choices
                    .iter()
                    .position(|c| c == value)
                    .ok_or_else(|| format!("amont inconnu : « {value} »"))?;
            }
            (Dialog::DiffWorktree(d), "file") => {
                let files = d.files.as_ref().ok_or("la liste n'est pas encore lue")?;
                d.selected = files
                    .iter()
                    .position(|(_, p)| p.to_string_lossy().replace('\\', "/") == value)
                    .ok_or_else(|| format!("fichier absent de la liste : « {value} »"))?;
            }
            (Dialog::DeleteBranch(d), "also_remote") => d.also_remote = flag()?,
            (Dialog::Discard(d), "delete_added") => d.delete_added = flag()?,
            (Dialog::Reset(d), "stash_first") => d.stash_first = flag()?,
            (Dialog::Reset(d), "mode") => {
                d.mode = match value {
                    "soft" => ResetUiMode::Soft,
                    "mixed" => ResetUiMode::Mixed,
                    "hard" => ResetUiMode::Hard,
                    other => return Err(format!("mode inconnu : « {other} » (soft, mixed, hard)")),
                };
                d.confirm_hard = false;
            }
            (dialog, other) => {
                return Err(format!(
                    "le dialogue « {} » n'a pas de champ « {other} »",
                    dialog.kind()
                ))
            }
        }
        Ok(())
    }

    pub fn state(&self) -> serde_json::Value {
        let buttons: Vec<_> = self
            .buttons()
            .iter()
            .map(|b| {
                serde_json::json!({ "key": b.key, "label": b.label, "enabled": b.enabled,
                    "primary": b.primary, "danger": b.danger, "tip": b.tip })
            })
            .collect();
        let fields = match self {
            Dialog::NewBranch(d) => serde_json::json!({
                "name": d.name, "start": d.start, "switch": d.switch,
                "problem": d.problem.as_ref().map(problem_text),
                "suggestion": suggestion(&d.problem),
            }),
            Dialog::Rename(d) => serde_json::json!({
                "old": d.old, "name": d.name, "upstream": d.upstream,
                "problem": d.problem.as_ref().map(problem_text),
            }),
            Dialog::SwitchOverwrite(d) => serde_json::json!({
                "target": d.target,
                "what": d.what,
                "verb": d.verb,
                "files": d.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
            }),
            Dialog::DetachedLeft(d) => serde_json::json!({
                "target": d.target, "head": d.head, "count": d.count, "subjects": d.subjects,
                "can_anyway": d.can_anyway,
            }),
            Dialog::SetUpstream(d) => serde_json::json!({
                "branch": d.branch, "choices": d.choices,
                "selected": d.choices.get(d.selected),
            }),
            Dialog::DiffWorktree(d) => serde_json::json!({
                "branch": d.branch, "loaded": d.files.is_some(), "error": d.error,
                "files": d.files.as_ref().map(|f| f.iter()
                    .map(|(s, p)| format!("{s} {}", p.display().to_string().replace('\\', "/")))
                    .collect::<Vec<_>>()),
                "selected": d.selected,
            }),
            Dialog::ForceSwitch(d) => serde_json::json!({
                "target": d.target,
                "files": d.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
            }),
            Dialog::UpdateMethod(d) => serde_json::json!({
                "branch": d.branch, "upstream": d.upstream, "remember": d.remember,
            }),
            Dialog::Push(d) => serde_json::json!({
                "branch": d.branch, "remote": d.remote, "remote_branch": d.remote_branch,
                "loaded": d.preview.is_some(),
                "new_branch": d.preview.as_ref().map(|p| p.new_branch),
                "sets_upstream": d.preview.as_ref().map(|p| p.sets_upstream),
                "protected": d.preview.as_ref().map(|p| p.protected),
                "remote_only": d.preview.as_ref().map(|p| p.remote_only),
                "outgoing": d.preview.as_ref().map(|p| p.outgoing.iter().map(|c| format!("{} {}", c.short_id, c.subject)).collect::<Vec<_>>()),
                "follow_tags": d.follow_tags, "confirm_force": d.confirm_force,
            }),
            Dialog::PushRejected(d) => serde_json::json!({ "known_behind": d.known_behind }),
            Dialog::DeleteBranch(d) => serde_json::json!({
                "name": d.name, "upstream": d.upstream, "also_remote": d.also_remote,
            }),
            Dialog::DeleteUnmerged(d) => serde_json::json!({
                "name": d.name, "commits": d.commits, "subjects": d.subjects,
            }),
            Dialog::DeleteRemote(d) => {
                serde_json::json!({ "remote": d.remote, "branch": d.branch })
            }
            Dialog::Reset(d) => serde_json::json!({
                "target": d.target, "mode": d.mode.key(), "stash_first": d.stash_first,
                "confirm_hard": d.confirm_hard,
                "commits_leaving": d.preview.as_ref().map(|p| p.commits_leaving),
                "unreachable": d.preview.as_ref().map(|p| p.unreachable),
                "pushed": d.preview.as_ref().map(|p| p.pushed),
                "local_changes": d.preview.as_ref().map(|p| p.local_changes.iter().map(|f| f.display().to_string()).collect::<Vec<_>>()),
            }),
            Dialog::Discard(d) => serde_json::json!({
                "delete_added": d.delete_added,
                "items": d.items.iter().map(|(n, w)| format!("{n} : {w}")).collect::<Vec<_>>(),
                "unsaved": d.unsaved,
            }),
            Dialog::CheckoutCommit(d) => serde_json::json!({ "id": d.id, "subject": d.subject }),
            Dialog::Message(d) => {
                serde_json::json!({ "title": d.title, "text": d.text, "error": d.error })
            }
        };
        serde_json::json!({ "kind": self.kind(), "title": self.title(), "fields": fields, "buttons": buttons })
    }

    // ----- Dessin ---------------------------------------------------------------

    /// Dessine le dialogue ; rend la clé du bouton pressé (Échap : `cancel`, Entrée : le bouton principal).
    pub fn show(&mut self, ctx: &Context, p: &Palette) -> Option<&'static str> {
        let title = self.title();
        let width = self.width();
        let mut answer: Option<&'static str> = None;
        let mut first_frame_focus = false;
        let modal = kit::modal("git-dialogue").show(ctx, |ui| {
            ui.set_width(width);
            ui.label(RichText::new(&title).strong().size(15.0).color(p.text));
            ui.add_space(6.0);
            first_frame_focus = self.body(ui, p, &mut answer);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let buttons = self.buttons();
                // Le bouton principal en premier, sauf les dialogues dont « Annuler » est le principal.
                for b in &buttons {
                    let response = if b.danger {
                        let text = RichText::new(&b.label).color(egui::Color32::WHITE);
                        ui.add_enabled(
                            b.enabled,
                            egui::Button::new(text).fill(p.error.gamma_multiply(0.85)),
                        )
                    } else {
                        kit::button(ui, p, &b.label, b.primary, b.enabled)
                    };
                    let response = match &b.tip {
                        Some(tip) => response.on_disabled_hover_text(tip),
                        None => response,
                    };
                    if response.clicked() {
                        answer = Some(b.key);
                    }
                }
            });
        });
        let _ = first_frame_focus;
        let buttons = self.buttons();
        let has_cancel = buttons.iter().any(|b| b.key == "cancel");
        let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if (escape || modal.should_close()) && answer.is_none() {
            answer = Some(if has_cancel {
                "cancel"
            } else if buttons.iter().any(|b| b.key == "back") {
                "back"
            } else {
                "close"
            });
        }
        if answer.is_none() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            // Entrée : le bouton principal s'il est actif.
            if let Some(b) = buttons.iter().find(|b| b.primary && b.enabled) {
                answer = Some(b.key);
            }
        }
        answer
    }

    fn body(&mut self, ui: &mut Ui, p: &Palette, answer: &mut Option<&'static str>) -> bool {
        let dim = |s: String| RichText::new(s).color(p.text_dim);
        match self {
            Dialog::NewBranch(d) => {
                ui.label(dim(t::LABEL_BRANCH_NAME.to_owned()));
                let edit = ui.add(
                    TextEdit::singleline(&mut d.name)
                        .desired_width(f32::INFINITY)
                        .hint_text(t::HINT_BRANCH_NAME)
                        .id_salt("nouvelle-branche-nom"),
                );
                if !edit.has_focus() && ui.memory(|m| m.focused().is_none()) {
                    edit.request_focus();
                }
                name_feedback(ui, p, &d.problem, answer);
                ui.add_space(6.0);
                ui.label(dim(t::dlg_start_from(&d.start_label)));
                ui.add_space(4.0);
                ui.checkbox(&mut d.switch, t::CHECK_SWITCH_TO_NEW);
            }
            Dialog::Rename(d) => {
                ui.label(dim(t::LABEL_NEW_NAME.to_owned()));
                let edit = ui.add(
                    TextEdit::singleline(&mut d.name)
                        .desired_width(f32::INFINITY)
                        .id_salt("renommer-branche-nom"),
                );
                if !edit.has_focus() && ui.memory(|m| m.focused().is_none()) {
                    edit.request_focus();
                }
                name_feedback(ui, p, &d.problem, answer);
                if let Some(upstream) = &d.upstream {
                    ui.add_space(6.0);
                    ui.label(dim(t::rename_upstream_note(upstream)));
                }
            }
            Dialog::SwitchOverwrite(d) => {
                ui.label(dim(if d.is_switch {
                    t::SWITCH_OVERWRITE_INTRO.to_owned()
                } else {
                    t::OVERWRITE_INTRO_OPERATION.to_owned()
                }));
                ui.add_space(4.0);
                file_list(ui, p, &d.files);
                ui.add_space(6.0);
                ui.label(dim(if d.is_switch {
                    t::SWITCH_OVERWRITE_STASH_NOTE.to_owned()
                } else {
                    t::overwrite_stash_note(&d.verb)
                }));
            }
            Dialog::DetachedLeft(d) => {
                ui.label(RichText::new(t::detached_left_intro(d.count)).color(p.error));
                ui.add_space(4.0);
                for subject in &d.subjects {
                    ui.label(RichText::new(format!("•  {subject}")).color(p.text));
                }
                if d.count as usize > d.subjects.len() {
                    ui.label(dim(t::and_n_more(d.count as usize - d.subjects.len())));
                }
                ui.add_space(6.0);
                ui.label(dim(t::DETACHED_LEFT_HELP.to_owned()));
                ui.label(dim(t::DETACHED_LEFT_CREATE_HELP.to_owned()));
            }
            Dialog::SetUpstream(d) => {
                ui.label(dim(t::UPSTREAM_PICK_HELP.to_owned()));
                ui.add_space(4.0);
                if d.choices.is_empty() {
                    ui.label(dim(t::UPSTREAM_PICK_NONE.to_owned()));
                }
                egui::ScrollArea::vertical()
                    .id_salt("amont-choix")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (i, choice) in d.choices.iter().enumerate() {
                            ui.radio_value(&mut d.selected, i, choice);
                        }
                    });
            }
            Dialog::DiffWorktree(d) => match (&d.files, &d.error) {
                (_, Some(error)) => {
                    ui.label(RichText::new(error).color(p.error));
                }
                (None, _) => {
                    ui.label(dim(t::DIFF_WORKTREE_LOADING.to_owned()));
                }
                (Some(files), _) if files.is_empty() => {
                    ui.label(dim(t::DIFF_WORKTREE_NONE.to_owned()));
                }
                (Some(files), _) => {
                    let mut selected = d.selected;
                    egui::ScrollArea::vertical()
                        .id_salt("diff-arbre-fichiers")
                        .max_height(260.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            for (i, (status, path)) in files.iter().enumerate() {
                                let label = format!("{status}   {}", path.display());
                                let response = ui.selectable_label(selected == i, label);
                                if response.clicked() {
                                    selected = i;
                                }
                                if response.double_clicked() {
                                    selected = i;
                                    *answer = Some("open_diff");
                                }
                            }
                        });
                    d.selected = selected;
                    ui.add_space(4.0);
                    ui.label(dim(t::DIFF_WORKTREE_HELP.to_owned()));
                }
            },
            Dialog::ForceSwitch(d) => {
                ui.label(RichText::new(t::FORCE_SWITCH_WARNING).color(p.error));
                ui.add_space(4.0);
                file_list(ui, p, &d.files);
                ui.add_space(6.0);
                ui.label(dim(t::FORCE_SWITCH_SAFETY.to_owned()));
            }
            Dialog::UpdateMethod(d) => {
                ui.label(dim(t::update_method_intro(
                    &d.branch,
                    d.upstream.as_deref(),
                )));
                ui.add_space(6.0);
                ui.label(dim(t::UPDATE_METHOD_MERGE_HELP.to_owned()));
                ui.label(dim(t::UPDATE_METHOD_REBASE_HELP.to_owned()));
                ui.add_space(6.0);
                ui.checkbox(&mut d.remember, t::CHECK_REMEMBER_METHOD);
                ui.label(
                    RichText::new(t::SETTINGS_REMEMBER_NOTE)
                        .small()
                        .color(p.text_dim),
                );
            }
            Dialog::Push(d) => push_body(ui, p, d),
            Dialog::PushRejected(d) => {
                ui.label(dim(t::push_rejected_text(d.known_behind)));
            }
            Dialog::DeleteBranch(d) => {
                ui.label(dim(t::DELETE_BRANCH_HELP.to_owned()));
                if let Some(upstream) = &d.upstream {
                    ui.add_space(6.0);
                    ui.checkbox(&mut d.also_remote, t::check_delete_remote_too(upstream));
                }
            }
            Dialog::DeleteUnmerged(d) => {
                ui.label(RichText::new(t::delete_unmerged_text(d.commits)).color(p.error));
                ui.add_space(4.0);
                for subject in &d.subjects {
                    ui.label(RichText::new(format!("•  {subject}")).color(p.text));
                }
                if d.commits as usize > d.subjects.len() {
                    ui.label(dim(t::and_n_more(d.commits as usize - d.subjects.len())));
                }
                ui.add_space(6.0);
                ui.label(dim(t::delete_unmerged_restore(&d.tip)));
            }
            Dialog::DeleteRemote(d) => {
                ui.label(RichText::new(t::delete_remote_text(&d.remote, &d.branch)).color(p.error));
            }
            Dialog::Reset(d) => reset_body(ui, p, d),
            Dialog::Discard(d) => {
                for (name, what) in d.items.iter().take(10) {
                    ui.label(RichText::new(format!("{name}  —  {what}")).color(p.text));
                }
                if d.items.len() > 10 {
                    ui.label(dim(t::and_n_more(d.items.len() - 10)));
                }
                for name in &d.unsaved {
                    ui.add_space(2.0);
                    ui.label(RichText::new(t::discard_unsaved(name)).color(p.warn));
                }
                if d.added > 0 {
                    ui.add_space(6.0);
                    ui.label(dim(t::discard_added_note(d.added)));
                    ui.checkbox(&mut d.delete_added, t::CHECK_DISCARD_DELETE_ADDED);
                }
                ui.add_space(6.0);
                ui.label(dim(t::DISCARD_SAFETY.to_owned()));
            }
            Dialog::CheckoutCommit(d) => {
                ui.label(dim(t::checkout_commit_text(&d.short, &d.subject)));
            }
            Dialog::Message(d) => {
                let color = if d.error { p.text } else { p.text_dim };
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(RichText::new(&d.text).color(color).monospace().small());
                    });
            }
        }
        false
    }
}

fn has_local(d: &Reset) -> bool {
    d.preview
        .as_ref()
        .is_some_and(|p| !p.local_changes.is_empty())
}

fn suggestion(problem: &Option<BranchNameProblem>) -> Option<String> {
    match problem {
        Some(BranchNameProblem::Invalid { suggestion }) => suggestion.clone(),
        _ => None,
    }
}

/// Le message du nom refusé, avec la proposition d'un clic.
fn name_feedback(
    ui: &mut Ui,
    p: &Palette,
    problem: &Option<BranchNameProblem>,
    answer: &mut Option<&'static str>,
) {
    let Some(problem) = problem else { return };
    ui.add_space(3.0);
    ui.label(RichText::new(problem_text(problem)).color(p.error));
    if let Some(suggestion) = suggestion(&Some(problem.clone())) {
        if ui.small_button(t::use_suggestion(&suggestion)).clicked() {
            *answer = Some("use_suggestion");
        }
    }
}

fn file_list(ui: &mut Ui, p: &Palette, files: &[PathBuf]) {
    for file in files.iter().take(10) {
        ui.label(RichText::new(format!("•  {}", file.display())).color(p.text));
    }
    if files.len() > 10 {
        ui.label(RichText::new(t::and_n_more(files.len() - 10)).color(p.text_dim));
    }
}

fn push_body(ui: &mut Ui, p: &Palette, d: &mut Push) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(&d.branch).strong().color(p.text));
        ui.label(RichText::new("→").color(p.text_dim));
        ui.add_sized(
            [110.0, 22.0],
            TextEdit::singleline(&mut d.remote).id_salt("push-distant"),
        );
        ui.label(RichText::new(":").color(p.text_dim));
        let edit = ui.add_sized(
            [220.0, 22.0],
            TextEdit::singleline(&mut d.remote_branch).id_salt("push-branche"),
        );
        if edit.changed() {
            d.dirty = true;
        }
        if let Some(preview) = &d.preview {
            if preview.new_branch {
                let (rect, _) = ui.allocate_exact_size(vec2(70.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 9.0, p.accent);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    t::PILL_NEW,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }
    });
    ui.add_space(6.0);
    let Some(preview) = &d.preview else {
        ui.label(RichText::new(t::GIT_LOADING).color(p.text_dim));
        return;
    };
    if d.confirm_force {
        ui.label(RichText::new(t::push_force_text(preview.remote_only)).color(p.error));
        ui.add_space(4.0);
        ui.label(RichText::new(t::PUSH_FORCE_LEASE_HELP).color(p.text_dim));
        return;
    }
    if preview.protected {
        ui.label(
            RichText::new(t::PUSH_PROTECTED_NOTE)
                .small()
                .color(p.text_dim),
        );
    }
    if preview.sets_upstream {
        ui.label(
            RichText::new(t::PUSH_SETS_UPSTREAM)
                .small()
                .color(p.text_dim),
        );
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(t::push_outgoing_title(preview.outgoing.len()))
            .strong()
            .color(p.text),
    );
    egui::ScrollArea::vertical()
        .id_salt("push-sortants")
        .max_height(210.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if preview.outgoing.is_empty() {
                ui.label(RichText::new(t::PUSH_NOTHING).color(p.text_dim));
            }
            for (i, c) in preview.outgoing.iter().enumerate() {
                commit_row(ui, p, c, d.selected == i);
            }
        });
    ui.add_space(4.0);
    ui.checkbox(&mut d.follow_tags, t::CHECK_FOLLOW_TAGS);
}

fn commit_row(ui: &mut Ui, p: &Palette, c: &CommitSummary, selected: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), egui::Sense::hover());
    if selected {
        ui.painter().rect_filled(rect, 3.0, p.accent_soft);
    }
    let clip = ui.painter().with_clip_rect(rect);
    clip.text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &c.short_id,
        egui::FontId::monospace(12.0),
        p.text_dim,
    );
    clip.text(
        egui::pos2(rect.left() + 70.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &c.subject,
        egui::FontId::proportional(13.5),
        p.text,
    );
    clip.text(
        egui::pos2(rect.right() - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        &c.author.name,
        egui::FontId::proportional(12.5),
        p.text_dim,
    );
}

fn reset_body(ui: &mut Ui, p: &Palette, d: &mut Reset) {
    ui.label(RichText::new(t::reset_target(&d.short, &d.subject)).color(p.text_dim));
    ui.add_space(6.0);
    for (mode, label, help) in [
        (ResetUiMode::Soft, t::RESET_SOFT, t::RESET_SOFT_HELP),
        (ResetUiMode::Mixed, t::RESET_MIXED, t::RESET_MIXED_HELP),
        (ResetUiMode::Hard, t::RESET_HARD, t::RESET_HARD_HELP),
    ] {
        let response = ui.radio_value(&mut d.mode, mode, label);
        if response.changed() {
            d.confirm_hard = false;
        }
        ui.label(RichText::new(help).small().color(p.text_dim));
    }
    ui.add_space(6.0);
    let Some(preview) = &d.preview else {
        ui.label(RichText::new(t::GIT_LOADING).color(p.text_dim));
        return;
    };
    ui.label(
        RichText::new(t::reset_leaving(
            preview.commits_leaving,
            preview.unreachable,
        ))
        .color(p.text),
    );
    if preview.pushed {
        ui.label(RichText::new(t::RESET_PUSHED_NOTE).color(p.warn));
    }
    if d.mode == ResetUiMode::Hard && !preview.local_changes.is_empty() {
        ui.add_space(4.0);
        if d.confirm_hard {
            ui.label(RichText::new(t::RESET_HARD_CONFIRM).color(p.error));
        }
        for f in preview.local_changes.iter().take(10) {
            ui.label(RichText::new(format!("•  {}", f.display())).color(p.text));
        }
        ui.checkbox(&mut d.stash_first, t::CHECK_STASH_FIRST);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_branch(name: &str, problem: Option<BranchNameProblem>, checked: Option<&str>) -> Dialog {
        Dialog::NewBranch(NewBranch {
            name: name.into(),
            start: None,
            start_label: "HEAD".into(),
            switch: true,
            problem,
            checked: checked.map(str::to_owned),
            sent: String::new(),
        })
    }

    #[test]
    fn create_button_waits_for_the_live_check_and_refuses_a_bad_name() {
        let dialog = new_branch("ma-branche", None, None);
        assert!(!dialog.buttons()[0].enabled, "pas encore vérifié");
        let dialog = new_branch("ma-branche", None, Some("ma-branche"));
        assert!(dialog.buttons()[0].enabled);
        let dialog = new_branch(
            "ma branche",
            Some(BranchNameProblem::Invalid {
                suggestion: Some("ma-branche".into()),
            }),
            Some("ma branche"),
        );
        assert!(!dialog.buttons()[0].enabled);
        assert!(dialog.buttons()[0].tip.is_some());
    }

    #[test]
    fn destructive_dialogs_default_to_cancel() {
        let force = Dialog::ForceSwitch(ForceSwitch {
            target: "x".into(),
            files: vec![],
            conflict: OverwriteConflict::default(),
        });
        let buttons = force.buttons();
        assert!(buttons[0].primary && buttons[0].key == "cancel");
        assert!(buttons[1].danger);
        let remote = Dialog::DeleteRemote(DeleteRemote {
            remote: "origin".into(),
            branch: "x".into(),
        });
        assert_eq!(remote.buttons()[0].key, "cancel");
        assert!(remote.buttons()[0].primary);
    }

    #[test]
    fn a_refused_merge_offers_to_stash_and_merge() {
        let dialog = Dialog::SwitchOverwrite(SwitchOverwrite {
            target: "feature/login".into(),
            files: vec![],
            conflict: OverwriteConflict::default(),
            can_force: false,
            what: "la fusion de feature/login".into(),
            verb: "fusionner".into(),
            is_switch: false,
        });
        assert_eq!(
            dialog.title(),
            "Des changements locaux gênent la fusion de feature/login"
        );
        let buttons = dialog.buttons();
        assert_eq!(buttons[0].label, "Mettre de côté et fusionner");
        assert!(
            buttons.iter().all(|b| b.key != "force"),
            "pas de forçage pour une fusion"
        );
    }

    #[test]
    fn leaving_a_detached_head_with_commits_asks_first() {
        let dialog = Dialog::DetachedLeft(DetachedLeft {
            target: "main".into(),
            head: "abc".into(),
            count: 2,
            subjects: vec!["a".into(), "b".into()],
            can_anyway: true,
        });
        let keys: Vec<&str> = dialog.buttons().iter().map(|b| b.key).collect();
        assert_eq!(keys, ["create_first", "anyway", "cancel"]);
        assert_eq!(dialog.title(), "2 commits ne sont sur aucune branche");
        assert_eq!(t::dlg_detached_left(1), "1 commit n'est sur aucune branche");
        assert!(
            dialog.buttons()[0].primary,
            "créer la branche est le choix sûr"
        );
    }

    #[test]
    fn setting_an_unknown_field_is_an_error() {
        let mut dialog = new_branch("a", None, None);
        assert!(dialog.set_field("name", "b").is_ok());
        assert!(dialog.set_field("remember", "on").is_err());
    }
}
