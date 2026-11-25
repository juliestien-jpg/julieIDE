//! Catalogue des textes de la v1 : branches, distant, opérations Git, menus
//! contextuels. **Provisoire** : `redacteur` le relit et le fusionne dans
//! `texts.rs` (écrit par dev-interface faute de relecture au moment du lot).
//! Aucun composant n'écrit de texte visible en dur ; tout passe par ici.

use julie_vcs::{BranchNameProblem, OperationKind, ProgressPhase, PushedKind, RemoteProgress};

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n > 1 { many } else { one })
}

/// `3 fichiers modifiés` / `1 fichier modifié` : le participe suit le nombre.
fn counted(n: usize, one: &str, many: &str, participle: &str) -> String {
    if n > 1 {
        format!("{n} {many} {participle}s")
    } else {
        format!("{n} {one} {participle}")
    }
}

fn operation_noun(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Merge => "la fusion",
        OperationKind::Rebase => "le rebase",
        OperationKind::CherryPick => "le cherry-pick",
        OperationKind::Revert => "le revert",
    }
}

// ----- Actions du registre ----------------------------------------------------------
pub const ACTION_UPDATE_PROJECT: &str = "Mettre à jour le projet…";
pub const ACTION_PUSH: &str = "Pousser…";
pub const ACTION_NEW_BRANCH: &str = "Nouvelle branche…";
pub const ACTION_BRANCHES: &str = "Branches…";
pub const ACTION_FETCH: &str = "Récupérer (fetch)";
pub const ACTION_CONTEXT_MENU: &str = "Ouvrir le menu contextuel";
pub const ACTION_GIT_ADD: &str = "Ajouter à Git";
pub const ACTION_GIT_UNSTAGE: &str = "Retirer de l'index";
pub const ACTION_GIT_DISCARD: &str = "Annuler les modifications…";
pub const ACTION_GIT_SHOW_DIFF: &str = "Voir le diff";
pub const ACTION_GIT_FILE_HISTORY: &str = "Historique du fichier";
pub const ACTION_IGNORE_FILE: &str = "Ajouter ce fichier au .gitignore";
pub const ACTION_IGNORE_FOLDER: &str = "Ajouter ce dossier au .gitignore";
pub const ACTION_IGNORE_EXTENSION: &str =
    "Ajouter tous les fichiers de cette extension au .gitignore";
pub const ACTION_OPEN_ENTRY: &str = "Ouvrir";
pub const ACTION_COPY_PATH: &str = "Chemin absolu";
pub const ACTION_COPY_RELATIVE_PATH: &str = "Chemin relatif au projet";
pub const ACTION_COPY_NAME: &str = "Nom du fichier";
pub const ACTION_REVEAL: &str = "Ouvrir dans l'Explorateur";
pub const ACTION_TERMINAL_HERE: &str = "Ouvrir un terminal ici";
pub const ACTION_FIND_IN_FOLDER: &str = "Rechercher dans ce dossier…";
pub const ACTION_TAB_CLOSE_OTHERS: &str = "Fermer les autres onglets";
pub const ACTION_TAB_CLOSE_ALL: &str = "Fermer tous les onglets";
pub const ACTION_TAB_CLOSE_UNMODIFIED: &str = "Fermer les onglets non modifiés";
pub const ACTION_LOG_NEW_BRANCH: &str = "Nouvelle branche ici…";
pub const ACTION_LOG_CHECKOUT: &str = "Basculer sur ce commit";
pub const ACTION_LOG_CHERRY_PICK: &str = "Cherry-pick";
pub const ACTION_LOG_REVERT: &str = "Annuler par un commit inverse (revert)";
pub const ACTION_LOG_RESET: &str = "Réinitialiser la branche ici…";
pub const ACTION_COPY_HASH: &str = "Copier le hash complet";
pub const ACTION_COPY_SHORT_HASH: &str = "Copier le hash court";
pub const ACTION_COPY_MESSAGE: &str = "Copier le message";
pub const ACTION_BRANCH_CHECKOUT: &str = "Basculer";
pub const ACTION_BRANCH_NEW_FROM: &str = "Nouvelle branche depuis celle-ci…";
pub const ACTION_BRANCH_CHECKOUT_REBASE: &str = "Basculer et rebaser sur la courante";
pub const ACTION_BRANCH_COMPARE: &str = "Comparer avec la courante";
pub const ACTION_BRANCH_REBASE_ONTO: &str = "Rebaser la courante sur celle-ci";
pub const ACTION_BRANCH_MERGE: &str = "Fusionner dans la courante";
pub const ACTION_BRANCH_UPDATE: &str = "Mettre à jour";
pub const ACTION_BRANCH_PULL_MERGE: &str = "Tirer dans la courante par fusion";
pub const ACTION_BRANCH_PULL_REBASE: &str = "Tirer dans la courante par rebase";
pub const ACTION_BRANCH_PUSH: &str = "Pousser…";
pub const ACTION_BRANCH_RENAME: &str = "Renommer…";
pub const ACTION_BRANCH_DELETE: &str = "Supprimer…";

pub const MENU_GIT: &str = "Git";
pub const MENU_COPY_PATH: &str = "Copier le chemin";
pub const MENU_IGNORE: &str = "Ajouter à .gitignore";
pub const LABEL_OPEN_FILE: &str = "Ouvrir le fichier";
pub const LABEL_CLOSE_TAB: &str = "Fermer";

pub fn label_checkout_rebase(current: &str) -> String {
    format!("Basculer et rebaser sur {current}")
}
pub fn label_compare(current: &str) -> String {
    format!("Comparer avec {current}")
}
pub fn label_rebase_onto(current: &str, onto: &str) -> String {
    format!("Rebaser {current} sur {onto}")
}
pub fn label_merge_into(branch: &str, current: &str) -> String {
    format!("Fusionner {branch} dans {current}")
}
pub fn label_pull_merge(current: &str) -> String {
    format!("Tirer dans {current} par fusion")
}
pub fn label_pull_rebase(current: &str) -> String {
    format!("Tirer dans {current} par rebase")
}
pub fn label_new_from(branch: &str) -> String {
    format!("Nouvelle branche depuis {branch}…")
}
pub fn label_ignore_ext(ext: &str) -> String {
    format!("Ajouter tous les *.{ext} au .gitignore")
}

// ----- Raisons des entrées grisées (infobulles) ----------------------------------------
pub const REASON_NOT_IN_REPO: &str = "Ce dossier n'est pas dans un dépôt Git.";
pub const REASON_GIT_LOADING: &str = "L'état Git est en cours de lecture.";
pub const REASON_NOTHING_TO_ADD: &str =
    "Rien à ajouter : aucun fichier non suivi ni modifié non indexé.";
pub const REASON_NOTHING_STAGED: &str = "Rien d'indexé dans la sélection.";
pub const REASON_NOTHING_TO_DISCARD: &str = "Aucun changement à annuler.";
pub const REASON_RESOLVE_CONFLICT: &str = "Résolvez d'abord le conflit.";
pub const REASON_ONE_FILE: &str = "Choisissez un seul fichier.";
pub const REASON_NO_CHANGE: &str = "Ce fichier n'a pas de changement.";
pub const REASON_UNTRACKED: &str = "Ce fichier n'est pas suivi par Git.";
pub const REASON_NO_COMMIT: &str = "Le dépôt n'a pas encore de commit.";
pub const REASON_ALREADY_TRACKED: &str = "Git suit déjà ce fichier : l'ignorer ne le retire pas.";
pub const REASON_ONE_TAB: &str = "Il n'y a qu'un onglet.";
pub const REASON_NO_UNMODIFIED_TAB: &str = "Tous les onglets sont modifiés.";
pub const REASON_FOLDER: &str = "Un dossier ne s'ouvre pas dans l'éditeur.";
pub const REASON_DELETED: &str = "Ce fichier est supprimé du disque.";
pub const REASON_MERGE_COMMIT: &str = "Commit de fusion : non pris en charge.";
pub const REASON_OPERATION: &str = "Terminez ou abandonnez l'opération en cours.";
pub const REASON_DETACHED: &str = "HEAD est détachée : pas de branche courante.";
pub const REASON_NO_UPSTREAM: &str = "Cette branche n'a pas de branche distante associée.";
pub const REASON_CURRENT_BRANCH: &str = "C'est la branche courante.";
pub const REASON_PROTECTED: &str = "Branche protégée.";

// ----- Widget et fenêtre surgissante des branches -----------------------------------------
pub const WIDGET_NO_REPO: &str = "Pas de dépôt Git";

pub fn widget_detached(short: &str) -> String {
    format!("HEAD détachée ({short})")
}

pub fn widget_operation(kind: OperationKind, step: Option<(u32, u32)>) -> String {
    let name = match kind {
        OperationKind::Merge => "Fusion",
        OperationKind::Rebase => "Rebase",
        OperationKind::CherryPick => "Cherry-pick",
        OperationKind::Revert => "Revert",
    };
    match step {
        Some((n, total)) => format!("{name} en cours ({n}/{total})"),
        None => format!("{name} en cours"),
    }
}

pub const BRANCH_SEARCH_HINT: &str = "Rechercher une branche…";
pub const BRANCH_NONE_FOUND: &str = "Aucune branche ne correspond.";
pub const HDR_ACTIONS: &str = "Actions";
pub const HDR_RECENT: &str = "Récentes";
pub const HDR_LOCAL: &str = "Locales";
pub const HDR_REMOTE: &str = "Distantes";
pub const TIP_FETCH: &str = "Récupérer : git fetch --all --prune";
pub const UPSTREAM_GONE_MARK: &str = "amont supprimé";
pub const TREE_ALL: &str = "Toutes les branches";

// ----- Dialogues : titres, boutons, textes ------------------------------------------------
pub const DLG_NEW_BRANCH: &str = "Nouvelle branche";
pub const DLG_FORCE_SWITCH: &str = "Forcer la bascule ?";
pub const DLG_UPDATE_METHOD: &str = "Mettre à jour le projet";
pub const DLG_PUSH_REJECTED: &str = "Le distant a des commits que vous n'avez pas";

pub fn dlg_rename(old: &str) -> String {
    format!("Renommer la branche {old}")
}
pub fn dlg_switch_overwrite(target: &str) -> String {
    format!("Des changements locaux gênent la bascule vers {target}")
}
pub fn dlg_push(branch: &str) -> String {
    format!("Pousser {branch}")
}
pub fn dlg_delete_branch(name: &str) -> String {
    format!("Supprimer la branche {name} ?")
}
pub fn dlg_delete_unmerged(name: &str) -> String {
    format!("{name} n'est pas fusionnée")
}
pub fn dlg_delete_remote(remote: &str, branch: &str) -> String {
    format!("Supprimer {remote}/{branch} sur le distant ?")
}
pub fn dlg_reset(branch: &str, short: &str) -> String {
    format!("Réinitialiser {branch} sur {short}")
}
pub fn dlg_discard(n: usize) -> String {
    format!(
        "Annuler les modifications de {} ?",
        plural(n, "fichier", "fichiers")
    )
}
pub fn dlg_checkout_commit(short: &str) -> String {
    format!("Basculer sur le commit {short} ?")
}
pub fn dlg_start_from(label: &str) -> String {
    format!("Depuis : {label}")
}

pub const BTN_CREATE: &str = "Créer";
pub const BTN_RENAME: &str = "Renommer";
pub const BTN_STASH_AND_SWITCH: &str = "Mettre de côté et basculer";
pub const BTN_FORCE_SWITCH: &str = "Forcer la bascule…";
pub const BTN_FORCE_AND_DISCARD: &str = "Forcer et jeter mes changements";
pub const BTN_METHOD_MERGE: &str = "Fusion";
pub const BTN_METHOD_REBASE: &str = "Rebase";
pub const BTN_PUSH: &str = "Pousser";
pub const BTN_FORCE_LEASE_ASK: &str = "Forcer avec bail…";
pub const BTN_FORCE_LEASE: &str = "Forcer avec bail";
pub const BTN_BACK: &str = "Retour";
pub const BTN_MERGE_THEN_PUSH: &str = "Fusionner puis pousser";
pub const BTN_REBASE_THEN_PUSH: &str = "Rebaser puis pousser";
pub const BTN_DELETE: &str = "Supprimer";
pub const BTN_DELETE_ANYWAY: &str = "Supprimer quand même";
pub const BTN_DELETE_REMOTE: &str = "Supprimer sur le distant";
pub const BTN_CONTINUE: &str = "Continuer…";
pub const BTN_RESET: &str = "Réinitialiser";
pub const BTN_DISCARD: &str = "Annuler les modifications";
pub const BTN_CHECKOUT: &str = "Basculer";
pub const BTN_OPEN_TERMINAL: &str = "Ouvrir le terminal";
pub const BTN_PUSH_TO_CREATE: &str = "Pousser pour la créer…";
pub const BTN_PUSH_TO_RECREATE: &str = "Pousser pour la recréer…";

pub const TIP_NAME_NOT_VALID: &str = "Le nom de la branche n'est pas valable.";
pub const TIP_NOTHING_TO_PUSH: &str = "Rien à pousser.";
pub const TIP_PROTECTED_BRANCH: &str =
    "Branche protégée : le push forcé est interdit (réglage [vcs] protected_branches).";

pub const LABEL_BRANCH_NAME: &str = "Nom de la branche";
pub const HINT_BRANCH_NAME: &str = "feature/ma-branche";
pub const LABEL_NEW_NAME: &str = "Nouveau nom";
pub const CHECK_SWITCH_TO_NEW: &str = "Basculer sur la nouvelle branche";

pub fn rename_upstream_note(upstream: &str) -> String {
    format!("La branche distante {upstream} garde son nom : seule la branche locale est renommée.")
}

pub fn use_suggestion(suggestion: &str) -> String {
    format!("Utiliser « {suggestion} »")
}

pub fn branch_name_problem(problem: &BranchNameProblem) -> String {
    match problem {
        BranchNameProblem::Empty => "Le nom est vide.".to_owned(),
        BranchNameProblem::Invalid { .. } => {
            "Ce nom n'est pas valable pour Git (espaces, « .. », ~ ^ : ? * [ \\, fin en « .lock » ou « / »)."
                .to_owned()
        }
        BranchNameProblem::AlreadyExists(name) => format!("La branche {name} existe déjà."),
        BranchNameProblem::SameNameDifferentCase(name) => format!(
            "La branche {name} porte déjà ce nom à la casse près : Windows ne les distingue pas."
        ),
        BranchNameProblem::FolderOfExisting(name) => format!(
            "Ce nom serait le dossier de la branche {name} : Git ne peut pas avoir les deux."
        ),
        BranchNameProblem::InsideExisting(name) => format!(
            "La branche {name} existe : elle ne peut pas aussi servir de dossier."
        ),
        BranchNameProblem::FolderCaseMismatch(name) => format!(
            "Un dossier de branches existe avec une autre casse ({name}) : Windows les confond."
        ),
    }
}

pub const SWITCH_OVERWRITE_INTRO: &str =
    "Ces fichiers ont des changements locaux que la bascule écraserait :";
pub const SWITCH_OVERWRITE_STASH_NOTE: &str = "« Mettre de côté et basculer » range vos changements (git stash), bascule, puis les remet. Si la remise entre en conflit, l'entrée reste dans git stash list.";
pub const FORCE_SWITCH_WARNING: &str =
    "Les changements locaux de ces fichiers seront jetés : ils ne reviendront pas.";
pub const FORCE_SWITCH_SAFETY: &str =
    "Une copie est gardée 7 jours dans .juliestorm/annulations/ avant d'écraser.";

pub fn update_method_intro(branch: &str, upstream: Option<&str>) -> String {
    match upstream {
        Some(up) => format!("Comment intégrer les changements de {up} dans {branch} ?"),
        None => format!("Comment intégrer les changements du distant dans {branch} ?"),
    }
}
pub const UPDATE_METHOD_MERGE_HELP: &str =
    "Fusion : un commit de fusion réunit les deux histoires.";
pub const UPDATE_METHOD_REBASE_HELP: &str =
    "Rebase : vos commits sont rejoués au-dessus de ceux du distant (historique linéaire).";
pub const CHECK_REMEMBER_METHOD: &str = "Ne plus demander";
pub const REMEMBER_SESSION_ONLY: &str =
    "Provisoire : le choix vaut pour cette session (le réglage [vcs] update_method viendra avec les réglages).";
pub const REMEMBER_SESSION_TOAST: &str =
    "Choix mémorisé pour cette session seulement (réglage non enregistré).";

pub const PILL_NEW: &str = "nouvelle";
pub const CHECK_FOLLOW_TAGS: &str = "Pousser aussi les étiquettes";
pub const PUSH_NOTHING: &str = "Rien à pousser.";
pub const PUSH_PROTECTED_NOTE: &str = "Branche protégée : le push forcé est interdit.";
pub const PUSH_SETS_UPSTREAM: &str =
    "Cette branche n'a pas d'amont : pousser le posera (git push -u).";
pub const PUSH_FORCE_LEASE_HELP: &str = "« Forcer avec bail » refuse d'écraser si le distant a bougé depuis votre dernière récupération : ce n'est jamais un push forcé nu.";
pub const PUSH_STALE_TITLE: &str = "Le distant a bougé";
pub const PUSH_STALE_TEXT: &str = "Le distant a changé depuis votre dernière récupération : rien n'a été écrasé. Récupérez, puis recommencez.";
pub const PUSH_REFUSED_TITLE: &str = "Push refusé";

pub fn push_outgoing_title(n: usize) -> String {
    if n == 0 {
        "Commits sortants".to_owned()
    } else {
        format!("Commits sortants ({n})")
    }
}
pub fn push_force_text(remote_only: u32) -> String {
    match remote_only {
        0 => "Aucun commit connu du distant ne sera remplacé.".to_owned(),
        n => format!(
            "{} du distant seront remplacés par votre branche.",
            plural(n as usize, "commit", "commits")
        ),
    }
}
pub fn push_rejected_text(known_behind: u32) -> String {
    let n = if known_behind > 0 {
        format!(" ({known_behind})")
    } else {
        String::new()
    };
    format!(
        "Le distant a des commits que vous n'avez pas encore{n}. Intégrez-les avant de pousser."
    )
}

pub const DELETE_BRANCH_HELP: &str = "Git refusera si la branche porte des commits non fusionnés.";

pub fn check_delete_remote_too(upstream: &str) -> String {
    format!("Supprimer aussi {upstream} sur le distant")
}
pub fn delete_unmerged_text(n: u32) -> String {
    format!(
        "{} ne seront plus sur aucune branche :",
        plural(n as usize, "commit", "commits")
    )
}
pub fn delete_unmerged_restore(tip: &str) -> String {
    let short: String = tip.chars().take(7).collect();
    format!("Récupérable : « Restaurer » juste après, ou git branch <nom> {short}.")
}
pub fn delete_remote_text(remote: &str, branch: &str) -> String {
    format!(
        "{remote}/{branch} sera supprimée sur le distant, pour tout le monde. Cette action ne s'annule pas depuis JulieStorm."
    )
}
pub fn and_n_more(n: usize) -> String {
    format!("… et {n} autres")
}

pub const RESET_SOFT: &str = "Garder tout indexé (--soft)";
pub const RESET_SOFT_HELP: &str =
    "HEAD recule ; les changements des commits retirés restent indexés.";
pub const RESET_MIXED: &str = "Garder les changements dans les fichiers (--mixed)";
pub const RESET_MIXED_HELP: &str =
    "HEAD recule ; les changements restent dans les fichiers, non indexés.";
pub const RESET_HARD: &str = "Tout jeter, fichiers compris (--hard)";
pub const RESET_HARD_HELP: &str =
    "HEAD recule et les fichiers reviennent à cet état : travail non validé perdu.";
pub const RESET_PUSHED_NOTE: &str =
    "Certains de ces commits sont déjà poussés : il faudra forcer le prochain push.";
pub const RESET_HARD_CONFIRM: &str = "Ces fichiers ont des changements locaux qui seront jetés :";
pub const CHECK_STASH_FIRST: &str = "Mettre de côté mes changements locaux avant";

pub fn reset_target(short: &str, subject: &str) -> String {
    format!("Cible : {short} — {subject}")
}
pub fn reset_leaving(n: u32, unreachable: u32) -> String {
    let verb = if n > 1 { "quittent" } else { "quitte" };
    let commits = plural(n as usize, "commit", "commits");
    if unreachable == 0 {
        format!("{commits} {verb} la branche (encore sur d'autres branches).")
    } else {
        format!(
            "{commits} {verb} la branche ; {unreachable} ne seront plus sur aucune branche (récupérables par le reflog de Git pendant 90 jours)."
        )
    }
}

pub const DISCARD_ADDED: &str = "retiré de l'index (le fichier reste sur le disque)";
pub const DISCARD_RENAMED: &str = "l'ancien chemin revient, le nouveau est retiré de l'index";
pub const DISCARD_DELETED: &str = "ramené à HEAD (le fichier revient)";
pub const DISCARD_MODIFIED: &str = "ramené à HEAD";
pub const DISCARD_SAFETY: &str =
    "Une copie est gardée 7 jours dans .juliestorm/annulations/ ; « Restaurer » la remet.";
pub const DISCARD_CONFLICTED: &str = "Un fichier est en conflit : résolvez-le d'abord.";

pub fn discard_unsaved(name: &str) -> String {
    format!("Les modifications non enregistrées de {name} seront perdues.")
}
pub fn discard_added_note(n: usize) -> String {
    format!(
        "{} sera retiré de l'index mais restera sur le disque.",
        plural(n, "fichier", "fichiers")
    )
}

pub fn checkout_commit_text(short: &str, subject: &str) -> String {
    format!(
        "{short} — {subject}\n\nVous serez en « HEAD détachée » : vos commits ne seront sur aucune branche tant que vous n'en créez pas une."
    )
}

pub const CONFIRM_SKIP_DETAIL: &str =
    "Les changements de ce commit ne seront pas repris dans la branche.";
pub const BTN_SKIP_COMMIT: &str = "Passer ce commit";
pub const BTN_CONTINUE_OP: &str = "Continuer";
pub const BTN_VALIDATE_MERGE: &str = "Valider";
pub const TIP_CONFLICTS_REMAIN: &str = "Il reste des fichiers en conflit.";

pub fn confirm_skip_title(kind: OperationKind) -> String {
    format!("Passer le commit en cours de {} ?", operation_noun(kind))
}
pub fn confirm_abort_detail(kind: OperationKind) -> String {
    match kind {
        OperationKind::Merge => {
            "Vos résolutions seront perdues ; la branche reviendra à son état d'avant la fusion."
                .to_owned()
        }
        OperationKind::Rebase => {
            "La branche reviendra à son état d'avant le rebase ; vos résolutions seront perdues."
                .to_owned()
        }
        _ => "Les changements de l'opération seront annulés ; vos résolutions seront perdues."
            .to_owned(),
    }
}
pub fn operation_abort_label(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Merge => "Abandonner la fusion",
        OperationKind::Rebase => "Abandonner le rebase",
        OperationKind::CherryPick => "Abandonner le cherry-pick",
        OperationKind::Revert => "Abandonner le revert",
    }
}
pub fn operation_resolved_continue(kind: OperationKind) -> String {
    format!(
        "Tous les conflits sont résolus : vous pouvez continuer {}.",
        operation_noun(kind)
    )
}
pub fn rebase_conflicts(step: (u32, u32), conflicts: usize) -> String {
    format!(
        "Rebase en cours — commit {} sur {} — {}",
        step.0,
        step.1,
        plural(conflicts, "fichier en conflit", "fichiers en conflit")
    )
}

pub const INDEX_STAGED: &str = "indexé";
pub const INDEX_PARTIAL: &str = "en partie indexé";

// ----- Filtres du journal ---------------------------------------------------------------------
pub const FILTER_TEXT_HINT: &str = "Texte";
pub const FILTER_AUTHOR_HINT: &str = "Auteur";
pub const FILTER_SINCE_HINT: &str = "Depuis AAAA-MM-JJ";
pub const FILTER_UNTIL_HINT: &str = "Jusqu'au AAAA-MM-JJ";
pub const TIP_REMOVE_FILTER: &str = "Retirer ce filtre";

pub fn filter_branch(branch: &str) -> String {
    format!("Branche : {branch}")
}
pub fn filter_path(name: &str) -> String {
    format!("Fichier : {name}")
}

// ----- Notifications ---------------------------------------------------------------------------
pub const UP_TO_DATE: &str = "Déjà à jour.";
pub const WAITING_FOR_GIT: &str = "En attente de l'opération Git en cours…";
pub const DETACHED_NOT_POSSIBLE: &str = "HEAD est détachée : créez d'abord une branche.";
pub const IGNORED_PATHS: &str = "Fichier ignoré par .gitignore : Git refuse de l'ajouter.";
pub const OPERATION_IN_PROGRESS: &str = "Terminez ou abandonnez l'opération en cours.";
pub const HOOK_FAILED_TITLE: &str = "Un crochet Git a échoué après la bascule";
pub const GIT_FAILED_TITLE: &str = "Git a refusé l'opération";
pub const TIP_CANCEL_TASK: &str = "Annuler cette opération";

pub const AUTH_TITLE: &str = "Authentification requise";
pub const AUTH_TEXT: &str = "Git demande une authentification qu'il ne peut pas obtenir ici. Lancez la commande une fois dans le terminal intégré pour enregistrer vos identifiants : JulieStorm ne demande ni ne garde jamais de mot de passe.";
pub const NO_UPSTREAM_TITLE: &str = "Pas de branche distante associée";
pub const UPSTREAM_GONE_TITLE: &str = "La branche distante n'existe plus";

pub fn no_upstream_text(branch: &str) -> String {
    format!("{branch} n'a pas de branche distante associée. Aucune fusion ni rebase n'a été lancé.")
}
pub fn upstream_gone_text(branch: &str, upstream: &str) -> String {
    format!("La branche distante {upstream} n'existe plus (supprimée sur le distant). {branch} reste intacte.")
}

pub fn already_on(name: &str) -> String {
    format!("Vous êtes déjà sur {name}.")
}
pub fn already_tracked(path: &str) -> String {
    format!("Git suit déjà {path} : l'ignorer ne le retire pas.")
}
pub fn protected_refused(branch: &str) -> String {
    format!("{branch} est une branche protégée : action refusée.")
}
pub fn branch_created(name: &str, switched: bool) -> String {
    if switched {
        format!("Branche {name} créée ; vous êtes dessus.")
    } else {
        format!("Branche {name} créée.")
    }
}
pub fn branch_renamed(old: &str, new: &str) -> String {
    format!("Branche {old} renommée en {new}.")
}
pub fn branch_deleted(name: &str) -> String {
    format!("Branche {name} supprimée.")
}
pub fn branch_restored(name: &str) -> String {
    format!("Branche {name} restaurée.")
}
pub fn remote_branch_deleted(remote: &str, branch: &str) -> String {
    format!("{remote}/{branch} supprimée sur le distant.")
}
pub fn fast_forwarded(branch: &str) -> String {
    format!("{branch} avancée sur son amont.")
}
pub fn switched(name: &str, created: bool, followed: bool) -> String {
    let base = if created {
        format!("Branche locale {name} créée et basculée")
    } else {
        format!("Basculé sur {name}")
    };
    if followed {
        format!("{base} ; vos changements locaux ont suivi.")
    } else {
        format!("{base}.")
    }
}
pub fn switched_behind(name: &str, behind: u32) -> String {
    format!(
        "Basculé sur {name}, en retard de {} sur son amont : utilisez « Mettre à jour le projet ».",
        plural(behind as usize, "commit", "commits")
    )
}
pub fn stash_kept(message: &str) -> String {
    format!("Remise en conflit : l'entrée « {message} » reste dans git stash list.")
}
pub fn fetched(updated: usize, pruned: usize, empty: bool) -> String {
    if empty {
        return "Rien de nouveau.".to_owned();
    }
    let mut parts = Vec::new();
    if updated > 0 {
        parts.push(if updated > 1 {
            format!("{updated} branches mises à jour")
        } else {
            "1 branche mise à jour".to_owned()
        });
    }
    if pruned > 0 {
        parts.push(if pruned > 1 {
            format!("{pruned} supprimées sur le distant")
        } else {
            "1 supprimée sur le distant".to_owned()
        });
    }
    format!("Récupéré : {}.", parts.join(", "))
}
pub fn integrated(kind: OperationKind, received: u32, changed: u32, fast_forward: bool) -> String {
    let what = match kind {
        OperationKind::Merge => "Fusion terminée",
        OperationKind::Rebase => "Rebase terminé",
        OperationKind::CherryPick => "Cherry-pick terminé",
        OperationKind::Revert => "Revert terminé",
    };
    let mut text = what.to_owned();
    if received > 0 {
        text.push_str(&format!(
            " : {}",
            counted(received as usize, "commit", "commits", "reçu")
        ));
    }
    if changed > 0 {
        text.push_str(&format!(
            ", {}",
            counted(changed as usize, "fichier", "fichiers", "changé")
        ));
    }
    if fast_forward {
        text.push_str(" (avance rapide)");
    }
    text.push('.');
    text
}
pub fn operation_stopped(kind: OperationKind, conflicts: usize) -> String {
    format!(
        "{} s'est arrêté{} : {} à résoudre.",
        match kind {
            OperationKind::Merge => "La fusion",
            OperationKind::Rebase => "Le rebase",
            OperationKind::CherryPick => "Le cherry-pick",
            OperationKind::Revert => "Le revert",
        },
        if kind == OperationKind::Merge {
            "e"
        } else {
            ""
        },
        plural(conflicts, "fichier en conflit", "fichiers en conflit")
    )
}
pub fn skipped(finished: bool) -> String {
    if finished {
        "Commit passé ; opération terminée.".to_owned()
    } else {
        "Commit passé ; un autre conflit à résoudre.".to_owned()
    }
}
pub fn pushed(n: u32, remote: &str, branch: &str, kind: PushedKind) -> String {
    match kind {
        PushedKind::UpToDate => format!("Rien à pousser : {remote}/{branch} est à jour."),
        PushedKind::Forced => format!("{remote}/{branch} remplacée (forcé avec bail)."),
        _ => format!(
            "{} vers {remote}/{branch}.",
            counted(n as usize, "commit", "commits", "poussé")
        ),
    }
}
pub fn progress_text(progress: &RemoteProgress) -> String {
    let phase = match &progress.phase {
        ProgressPhase::Enumerating => "Énumération",
        ProgressPhase::Counting => "Comptage",
        ProgressPhase::Compressing => "Compression",
        ProgressPhase::Receiving => "Réception",
        ProgressPhase::Resolving => "Résolution",
        ProgressPhase::Writing => "Envoi",
        ProgressPhase::Checkout => "Extraction",
        ProgressPhase::Other(text) => text.as_str(),
    };
    match progress.percent {
        Some(p) => format!("{phase} {p} %"),
        None => phase.to_owned(),
    }
}
pub fn reset_done(head: &str, stashed: bool) -> String {
    let short: String = head.chars().take(7).collect();
    if stashed {
        format!("Branche réinitialisée sur {short} ; vos changements sont dans git stash.")
    } else {
        format!("Branche réinitialisée sur {short}.")
    }
}
pub fn staged(n: usize) -> String {
    format!("{} à l'index.", counted(n, "fichier", "fichiers", "ajouté"))
}
pub fn unstaged(n: usize) -> String {
    format!(
        "{} de l'index.",
        counted(n, "fichier", "fichiers", "retiré")
    )
}
pub fn discarded(restored: usize, unstaged: usize) -> String {
    match (restored, unstaged) {
        (r, 0) => format!("{} à HEAD.", counted(r, "fichier", "fichiers", "ramené")),
        (0, u) => format!(
            "{} de l'index.",
            counted(u, "fichier", "fichiers", "retiré")
        ),
        (r, u) => format!(
            "{} à HEAD, {} de l'index.",
            counted(r, "fichier", "fichiers", "ramené"),
            counted(u, "fichier", "fichiers", "retiré")
        ),
    }
}
pub fn ignore_added(line: &str, already: bool) -> String {
    if already {
        format!("« {line} » est déjà dans .gitignore.")
    } else {
        format!("« {line} » ajouté à .gitignore.")
    }
}
pub fn copied(n: usize) -> String {
    if n > 1 {
        format!("{n} chemins copiés.")
    } else {
        "Copié.".to_owned()
    }
}
pub fn safety_failed(error: &str) -> String {
    format!("Copie de sécurité impossible, rien n'a été touché : {error}")
}
pub fn restored_files(n: usize) -> String {
    format!("{}.", counted(n, "fichier", "fichiers", "rétabli"))
}
pub fn restore_failed(error: &str) -> String {
    format!("Rétablissement impossible : {error}")
}
pub const BTN_RESTORE: &str = "Restaurer";

// ----- Fermer des onglets ------------------------------------------------------------------------
pub fn close_many_title(n: usize) -> String {
    format!("Fermer {} modifié(s) ?", plural(n, "onglet", "onglets"))
}

pub fn widget_tooltip(
    branch: &str,
    upstream: Option<&str>,
    ahead_behind: Option<(u32, u32)>,
    fetched_secs: Option<u64>,
) -> String {
    let mut text = format!("Branche : {branch}");
    match (upstream, ahead_behind) {
        (Some(up), Some((a, b))) => text.push_str(&format!(
            "
Amont : {up} (avance {a}, retard {b})"
        )),
        (Some(up), None) => text.push_str(&format!(
            "
Amont : {up} (supprimé sur le distant)"
        )),
        (None, _) => text.push_str(
            "
Pas d'amont",
        ),
    }
    text.push_str(&match fetched_secs {
        None => "
Dernier fetch : jamais (depuis l'ouverture)"
            .to_owned(),
        Some(s) if s < 90 => "
Dernier fetch : à l'instant"
            .to_owned(),
        Some(s) => format!(
            "
Dernier fetch : il y a {} min",
            s / 60
        ),
    });
    text
}

pub fn operation_done(kind: OperationKind) -> String {
    match kind {
        OperationKind::Merge => "Fusion terminée.",
        OperationKind::Rebase => "Rebase terminé.",
        OperationKind::CherryPick => "Cherry-pick terminé.",
        OperationKind::Revert => "Revert terminé.",
    }
    .to_owned()
}
