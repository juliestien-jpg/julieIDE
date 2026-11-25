//! Catalogue des textes de l'interface (français). **Provisoire** : `redacteur`
//! le relit et le complète. Aucun composant n'écrit de texte visible en dur ; tout
//! passe par ce module, regroupé par écran. Les textes qui portent des valeurs
//! sont des fonctions.
//!
//! (Les textes du terminal et de l'éditeur vivent dans leurs modules `texts`, qui
//! seront rapatriés ici avec le catalogue.)

use std::path::Path;
use std::time::Duration;

use julie_phpunit::{Counts, Problem, RunOutcome, RunSummary, TestState};
use julie_vcs::{
    BranchNameProblem, ConflictKind, ConflictSide, FileKind, Head, OperationKind, ProgressPhase,
    PushedKind, RemoteProgress, Upstream,
};

pub const APP_NAME: &str = "JulieStorm";

// ----- Menus et actions ----------------------------------------------------------
pub const MENU_FILE: &str = "Fichier";
pub const MENU_NAVIGATE: &str = "Aller à";
pub const MENU_VIEW: &str = "Affichage";
pub const MENU_GIT: &str = "Git";
pub const MENU_TESTS: &str = "Tests";

pub const ACTION_OPEN_FOLDER: &str = "Ouvrir un dossier…";
pub const ACTION_SAVE_ALL: &str = "Tout enregistrer";
pub const ACTION_CLOSE_TAB: &str = "Fermer l'onglet";
pub const ACTION_RELOAD_DISK: &str = "Recharger depuis le disque";
pub const ACTION_QUIT: &str = "Quitter";
pub const ACTION_GOTO_FILE: &str = "Aller au fichier…";
pub const ACTION_FIND_IN_FILES: &str = "Rechercher dans les fichiers…";
pub const ACTION_PALETTE: &str = "Rechercher une action…";
pub const ACTION_PREVIOUS_TAB: &str = "Onglet précédent";
pub const ACTION_NEXT_TAB: &str = "Onglet suivant";
pub const ACTION_LAST_TAB: &str = "Revenir à l'onglet précédemment actif";
pub const ACTION_TOGGLE_PROJECT: &str = "Projet";
pub const ACTION_TOGGLE_GIT: &str = "Git";
pub const ACTION_TOGGLE_TESTS: &str = "Tests";
pub const ACTION_TOGGLE_SEARCH: &str = "Résultats de recherche";
pub const ACTION_TOGGLE_TERMINAL: &str = "Terminal";
pub const ACTION_TOGGLE_THEME: &str = "Changer de thème (sombre / clair)";
pub const ACTION_TOGGLE_COMMIT: &str = "Panneau de validation";
pub const ACTION_BOTTOM_TALL: &str = "Agrandir / réduire la fenêtre du bas";
pub const ACTION_GIT_COMMIT: &str = "Valider les changements…";
pub const ACTION_GIT_REFRESH: &str = "Actualiser l'état Git";
pub const ACTION_TESTS_RUN: &str = "Lancer les tests";
pub const ACTION_TESTS_STOP: &str = "Arrêter les tests";
pub const ACTION_TESTS_RERUN: &str = "Relancer les tests";

// ----- Accueil, projet, arbre -------------------------------------------------------
pub const WELCOME_NO_PROJECT: &str =
    "Aucun projet ouvert. Ouvrez un dossier depuis le menu Fichier.";
pub const PICK_FOLDER_TITLE: &str = "Ouvrir un dossier";
pub const TREE_NO_PROJECT: &str = "Aucun projet ouvert. Ouvrez un dossier depuis le menu Fichier.";
pub const TERMINAL_NO_PROJECT: &str = "Ouvrez un projet pour lancer un terminal dans son dossier.";

pub fn project_open_failed(path: &Path, error: &str) -> String {
    format!(
        "Impossible d'ouvrir le dossier {} : {error}. Vérifiez qu'il existe et que vous pouvez le lire.",
        path.display()
    )
}

pub fn settings_failed(error: &str) -> String {
    format!("Réglages illisibles, valeurs par défaut utilisées : {error}")
}

// ----- Onglets, éditeur ---------------------------------------------------------------
pub const UNTITLED: &str = "Sans titre";
pub const BANNER_CHANGED: &str = "Le fichier a changé sur le disque.";
pub const BANNER_DELETED: &str = "Le fichier a été supprimé du disque. L'enregistrer le recrée.";
pub const BANNER_RELOAD: &str = "Recharger";
pub const BANNER_RELOAD_CONFIRM: &str = "Perdre mes modifications ?";
pub const BANNER_KEEP: &str = "Garder les miennes";
pub const BANNER_DISMISS: &str = "Fermer ce message";
pub const EOL_MIXED: &str = "mixte";
pub const ERR_BINARY: &str = "Fichier binaire : impossible à ouvrir dans l'éditeur.";
pub const ERR_TOO_LARGE: &str = "Fichier trop gros pour l'éditeur.";

pub fn load_failed(path: &Path, reason: &str) -> String {
    format!("{} : {reason}", path.display())
}

pub fn save_failed(path: &Path, reason: &str) -> String {
    format!(
        "Enregistrement impossible pour {} : {reason}. Le fichier sur le disque n'a pas été modifié : réessayez, ou fermez le programme qui l'utilise.",
        path.display()
    )
}

pub fn git_failed(error: &str) -> String {
    format!("Git : {error}")
}

pub const DIALOG_SAVE_MANY: &str = "Enregistrer les modifications de ces fichiers ?";
pub const DIALOG_SAVE: &str = "Enregistrer";
pub const DIALOG_SAVE_ALL: &str = "Tout enregistrer";
pub const DIALOG_DISCARD: &str = "Ne pas enregistrer";
pub const DIALOG_CANCEL: &str = "Annuler";

pub fn dialog_save_one(name: &str) -> String {
    format!("Enregistrer les modifications de {name} ?")
}

// ----- Fenêtres d'outils -------------------------------------------------------------------
pub const TOOL_PROJECT: &str = "Projet";
pub const TOOL_GIT: &str = "Git";
pub const TOOL_TESTS: &str = "Tests";
pub const TOOL_SEARCH: &str = "Recherche";
pub const TOOL_TERMINAL: &str = "Terminal";
pub const TOOL_COLLAPSE: &str = "Replier la fenêtre";
pub const CLOSE: &str = "Fermer";

// ----- Aller au fichier / palette ------------------------------------------------------------
pub const GOTO_TITLE: &str = "Aller au fichier";
pub const GOTO_HINT: &str = "Nom du fichier (ex. usct, src/ctrl, index.php:12)";
pub const GOTO_SEARCHING: &str = "Recherche…";
pub const GOTO_INDEXING: &str = "Indexation… résultats partiels.";
pub const GOTO_NO_RECENT: &str = "Aucun fichier récent. Tapez un nom pour chercher.";
pub const GOTO_NONE: &str = "Aucun fichier ne correspond";

pub fn goto_more(n: usize) -> String {
    format!("… et {n} autres")
}

pub const PALETTE_TITLE: &str = "Rechercher une action";
pub const PALETTE_HINT: &str = "Nom de l'action";
pub const PALETTE_NONE: &str = "Aucune action ne correspond";

// ----- Recherche dans les fichiers ---------------------------------------------------------------
pub const FIND_TITLE: &str = "Rechercher dans les fichiers";
pub const FIND_HINT: &str = "Texte à chercher (2 caractères au moins)";
pub const FIND_CASE: &str = "Respecter la casse (Alt+C)";
pub const FIND_WORD: &str = "Mots entiers (Alt+W)";
pub const FIND_REGEX: &str = "Expression régulière (Alt+X)";
pub const FIND_MASK: &str = "Fichiers";
pub const FIND_MASK_HINT: &str = "*.php, *.js, !*.min.js";
pub const FIND_SCOPE: &str = "Portée";
pub const FIND_SCOPE_PROJECT: &str = "Projet";
pub const FIND_INVALID_REGEX: &str = "Expression régulière invalide";
pub const FIND_MIN_CHARS: &str = "Saisissez au moins 2 caractères.";

/// Message complet pour une expression régulière refusée : le début est toujours
/// français ; la cause est celle du crate `regex`, traduite, avec un repli
/// lisible pour les erreurs que la table ne connaît pas.
pub fn find_invalid_regex(raw: &str) -> String {
    let cause = raw
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map_or(raw, |l| l.trim().trim_start_matches("error:").trim());
    format!("{FIND_INVALID_REGEX} : {}", regex_cause_fr(cause))
}

/// Traduit la cause d'une erreur de syntaxe du crate `regex`.
pub fn regex_cause_fr(cause: &str) -> String {
    let lower = cause.to_lowercase();
    const TABLE: &[(&str, &str)] = &[
        (
            "unclosed capture group name",
            "nom de groupe de capture non fermé",
        ),
        (
            "unclosed group",
            "groupe non fermé (il manque une parenthèse fermante)",
        ),
        (
            "unopened group",
            "parenthèse fermante en trop (aucun groupe ouvert)",
        ),
        (
            "unclosed character class",
            "classe de caractères non fermée (il manque un crochet fermant)",
        ),
        (
            "unclosed counted repetition",
            "répétition {n,m} non fermée (il manque une accolade fermante)",
        ),
        (
            "repetition operator missing expression",
            "répétition sans rien à répéter (*, + ou ? ne peut pas commencer le motif)",
        ),
        (
            "repetition quantifier expects a valid decimal",
            "la répétition {…} attend un nombre",
        ),
        (
            "invalid repetition count range",
            "intervalle de répétition invalide (le minimum dépasse le maximum)",
        ),
        (
            "unrecognized escape sequence",
            "séquence d'échappement inconnue (caractère inattendu après la barre oblique inverse)",
        ),
        (
            "incomplete escape sequence",
            "échappement incomplet (le motif se termine par une barre oblique inverse)",
        ),
        (
            "invalid escape sequence found in character class",
            "échappement invalide dans une classe de caractères",
        ),
        (
            "invalid character class range",
            "intervalle invalide dans la classe (le début dépasse la fin)",
        ),
        (
            "invalid range boundary",
            "borne d'intervalle invalide (un caractère simple est attendu)",
        ),
        ("hexadecimal literal empty", "nombre hexadécimal vide"),
        ("invalid hexadecimal digit", "chiffre hexadécimal invalide"),
        ("invalid hexadecimal literal", "nombre hexadécimal invalide"),
        ("decimal literal empty", "nombre vide"),
        ("invalid decimal literal", "nombre invalide"),
        (
            "dangling flag negation",
            "option « - » sans option derrière",
        ),
        ("flag negation operator repeated", "option « - » répétée"),
        ("duplicate flag", "option en double"),
        (
            "unrecognized flag",
            "option inconnue (les options valides sont i, m, s, U, u, x)",
        ),
        (
            "expected flag but got end of regex",
            "option attendue, mais le motif se termine",
        ),
        (
            "duplicate capture group name",
            "nom de groupe de capture en double",
        ),
        ("empty capture group name", "nom de groupe de capture vide"),
        (
            "invalid capture group",
            "caractère invalide dans un nom de groupe de capture",
        ),
        ("special word boundary", "limite de mot spéciale mal écrite"),
        (
            "backreferences are not supported",
            "les références arrière (\\1) ne sont pas prises en charge",
        ),
        (
            "look-around",
            "les assertions avant et arrière ((?=…), (?<=…)) ne sont pas prises en charge",
        ),
        (
            "nested parentheses",
            "trop de parenthèses ou de crochets imbriqués",
        ),
        ("capturing groups", "trop de groupes de capture"),
        ("size limit", "expression trop complexe"),
        ("unicode", "propriété ou classe Unicode inconnue"),
        ("empty character class", "classe de caractères vide"),
    ];
    for (needle, fr) in TABLE {
        if lower.contains(needle) {
            return (*fr).to_owned();
        }
    }
    "syntaxe non reconnue".to_owned()
}
pub const FIND_RUNNING: &str = "recherche en cours";
pub const FIND_LIMIT: &str = "Plus de 10 000 résultats, précisez la recherche";
pub const FIND_NO_PREVIEW: &str = "Sélectionnez un résultat pour voir son contexte.";
pub const FIND_TAB_EMPTY: &str = "Aucune recherche. Ctrl+Maj+F pour chercher dans les fichiers.";

pub fn find_scope_dir(name: &str) -> String {
    format!("Dossier : {name}")
}

pub fn find_summary(matches: usize, files: usize) -> String {
    let occurrences = if matches > 1 {
        "occurrences"
    } else {
        "occurrence"
    };
    let fichiers = if files > 1 { "fichiers" } else { "fichier" };
    format!("{matches} {occurrences} dans {files} {fichiers}")
}

pub fn find_too_large(n: usize) -> String {
    if n > 1 {
        format!("{n} fichiers trop gros ignorés")
    } else {
        "1 fichier trop gros ignoré".to_owned()
    }
}

// ----- Barre d'état ----------------------------------------------------------------------------------
pub const STATUS_NO_FILE: &str = "Aucun fichier ouvert";
pub const STATUS_BRANCH_TOOLTIP: &str = "Branche Git courante";

/// Libellé d'une tâche de fond d'après sa clé `domaine.action`.
pub fn task_label(kind: &'static str) -> &'static str {
    match kind {
        "index.scan" => "Indexation du projet",
        "index.update" => "Mise à jour de l'index",
        "index.search_text" => "Recherche dans les fichiers",
        "text.load" => "Ouverture d'un fichier",
        "text.save" => "Enregistrement",
        "vcs.open" => "Détection du dépôt Git",
        "vcs.status" => "Lecture de l'état Git",
        "vcs.diff" => "Lecture du diff",
        "vcs.conflict" => "Lecture des versions du conflit",
        "vcs.resolve" => "Enregistrement de la fusion",
        "vcs.accept_side" => "Résolution du conflit",
        "vcs.continue" => "Conclusion de l'opération Git",
        "vcs.abort" => "Abandon de l'opération Git",
        "vcs.commit_details" => "Lecture du commit",
        "diff.compute" => "Calcul du diff",
        "diff.merge" => "Préparation de la fusion",
        "phpunit.detect" => "Détection de PHPUnit",
        "vcs.commit" => "Commit",
        "vcs.log" => "Lecture du journal Git",
        "vcs.branches" => "Lecture des branches",
        "vcs.fetch" => "Récupération (fetch)",
        "vcs.push" => "Envoi (push)",
        "vcs.update" => "Mise à jour du projet",
        "vcs.pull" => "Récupération et intégration (pull)",
        "vcs.switch" => "Bascule de branche",
        "vcs.branch_create" => "Création de la branche",
        "vcs.branch_update" => "Avance de la branche",
        "vcs.remote_branch_delete" => "Suppression de la branche distante",
        "vcs.merge" => "Fusion",
        "vcs.rebase" => "Rebase",
        "vcs.cherry_pick" => "Cherry-pick",
        "vcs.revert" => "Revert",
        "vcs.reset" => "Réinitialisation de la branche",
        "vcs.stage" | "vcs.unstage" => "Mise à jour de l'index",
        "vcs.discard" => "Annulation des modifications",
        "terminal.profiles" => "Détection des terminaux",
        "phpunit.run" => "Exécution des tests",
        "index.symbols" => "Indexation des symboles",
        "index.unsaved" => "Prise en compte du texte non enregistré",
        "index.usages" => "Recherche des usages",
        "index.symbols_cache" => "Écriture du cache des symboles",
        other => other,
    }
}

pub fn files_count(n: u64) -> String {
    if n > 1 {
        format!("{n} fichiers")
    } else {
        format!("{n} fichier")
    }
}

// ----- Diff côte à côte -----------------------------------------------------------------------
pub const DIFF_IDENTICAL: &str = "Les deux versions sont identiques.";
pub const DIFF_TOO_LARGE: &str = "Fichier trop gros pour le diff.";
pub const DIFF_UNSUPPORTED: &str = "Ce contenu (sous-module ou dossier) ne s'affiche pas en diff.";
pub const DIFF_LOADING: &str = "Chargement du diff…";
pub const DIFF_NO_SELECTION: &str = "Sélectionnez un fichier pour voir ses changements.";
pub const DIFF_PREVIOUS: &str = "Différence précédente (Maj+F7)";
pub const DIFF_NEXT: &str = "Différence suivante (F7)";
pub const DIFF_FINAL_NEWLINE_ADDED: &str = "Saut de ligne ajouté en fin de fichier";
pub const DIFF_FINAL_NEWLINE_REMOVED: &str = "Saut de ligne retiré en fin de fichier";
pub const DIFF_REV_ABSENT: &str = "(absent)";
pub const DIFF_REV_HEAD: &str = "Version validée (HEAD)";
pub const DIFF_REV_INDEX: &str = "Index";
pub const DIFF_REV_STAGE: &str = "Version de l'index";
pub const DIFF_REV_WORKTREE: &str = "Copie de travail";
pub const DIFF_CLOSE_HINT: &str =
    "Échap pour fermer — F7 / Maj+F7 : différence suivante / précédente";
pub const COMPARE_EXPECTED: &str = "Attendu";
pub const COMPARE_ACTUAL: &str = "Obtenu";

pub fn diff_rev_commit(short: &str) -> String {
    format!("Commit {short}")
}

pub fn diff_count(n: usize) -> String {
    match n {
        0 => "Aucune différence".to_owned(),
        1 => "1 différence".to_owned(),
        n => format!("{n} différences"),
    }
}

pub fn diff_only_line_endings(from: &str, to: &str) -> String {
    format!("Seules les fins de ligne diffèrent ({from} → {to}).")
}

pub fn diff_binary(left: Option<u64>, right: Option<u64>) -> String {
    let size = |n: Option<u64>| n.map_or("—".to_owned(), crate::kit::format_size);
    format!(
        "Fichiers binaires — {} avant / {} après",
        size(left),
        size(right)
    )
}

// ----- Fenêtre Git -------------------------------------------------------------------------------
pub const GIT_TAB_CHANGES: &str = "Changements";
pub const GIT_TAB_LOG: &str = "Journal";
pub const GIT_TAB_CONSOLE: &str = "Console";
pub const GIT_LOADING: &str = "Lecture de l'état Git…";
pub const GIT_NO_REPOSITORY: &str = "Ce dossier n'est pas un dépôt Git.";
pub const GIT_CLEAN: &str = "Aucun changement";
pub const GIT_REFRESH: &str = "Actualiser (Ctrl+Alt+Y)";
pub const GIT_GROUP_CONFLICTS: &str = "En conflit";
pub const GIT_GROUP_CHANGED: &str = "Modifiés";
pub const GIT_GROUP_UNTRACKED: &str = "Non suivis";
pub const GIT_CONFLICT_HINT: &str =
    "Fichier en conflit : utilisez « Fusionner… », ou gardez l'une des deux versions.";
pub const LOG_EMPTY: &str = "Aucun commit pour l'instant.";
pub const LOG_SELECT: &str = "Sélectionnez un commit pour voir son message et ses fichiers.";
pub const CONSOLE_EMPTY: &str = "Aucune commande Git lancée pour l'instant.";
pub const CONSOLE_RUNNING: &str = "en cours…";
pub const CONSOLE_OK: &str = "réussie";
pub const CONSOLE_NO_CODE: &str = "interrompue";

pub fn console_exit(code: i32) -> String {
    format!("code de sortie {code}")
}

pub fn git_head_summary(head: &Head, upstream: Option<&Upstream>) -> String {
    let mut text = match head {
        Head::Branch { name, .. } => name.clone(),
        Head::Detached { commit } => {
            format!(
                "HEAD détachée ({})",
                commit.chars().take(7).collect::<String>()
            )
        }
    };
    if let Some(upstream) = upstream {
        text.push_str(&format!("  →  {}", upstream.name));
        match upstream.ahead_behind {
            Some((0, 0)) => {}
            Some((ahead, behind)) => text.push_str(&format!("  (avance {ahead}, retard {behind})")),
            None => text.push_str("  (amont supprimé)"),
        }
    }
    text
}

pub fn file_kind(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Modified => "modifié",
        FileKind::Added => "ajouté",
        FileKind::Deleted => "supprimé",
        FileKind::Renamed => "renommé",
        FileKind::Copied => "copié",
        FileKind::TypeChanged => "type changé",
        FileKind::Untracked => "non suivi",
        FileKind::Conflicted => "en conflit",
    }
}

pub fn conflict_kind(kind: ConflictKind) -> &'static str {
    match kind {
        ConflictKind::BothModified => "modifié des deux côtés",
        ConflictKind::BothAdded => "ajouté des deux côtés",
        ConflictKind::BothDeleted => "supprimé des deux côtés",
        ConflictKind::AddedByUs => "ajouté par nous",
        ConflictKind::AddedByThem => "ajouté par eux",
        ConflictKind::DeletedByUs => "supprimé par nous",
        ConflictKind::DeletedByThem => "supprimé par eux",
    }
}

pub fn keep_labels(kind: Option<ConflictKind>) -> (&'static str, &'static str) {
    match kind {
        Some(ConflictKind::DeletedByThem) => ("Garder le fichier", "Le supprimer"),
        Some(ConflictKind::DeletedByUs) => ("Le supprimer", "Garder le fichier"),
        _ => ("Garder la nôtre", "Prendre la leur"),
    }
}

// ----- Validation ---------------------------------------------------------------------------------
pub const COMMIT_HINT: &str = "Message de validation";
pub const COMMIT_BUTTON: &str = "Valider";
pub const COMMIT_AMEND_BUTTON: &str = "Amender";
pub const COMMIT_CONCLUDE: &str = "Conclure";
pub const COMMIT_AMEND: &str = "Amender le dernier commit";
pub const COMMIT_FAILED: &str = "La validation a échoué";
pub const COMMIT_FAILED_DETAIL: &str =
    "Rien n'a été validé, votre message est conservé. Sortie de Git :";
pub const OPERATION_RESOLVED: &str = "Tous les conflits sont résolus, vous pouvez valider.";
pub const OPERATION_RESOLVE: &str = "Résoudre";
pub const CONFIRM_YES: &str = "Confirmer";

pub fn commit_done(subject: &str) -> String {
    format!("1 commit : {subject}")
}

pub fn commit_selected(n: usize) -> String {
    match n {
        0 => "Aucun fichier coché".to_owned(),
        1 => "1 fichier coché".to_owned(),
        n => format!("{n} fichiers cochés"),
    }
}

pub fn commit_tooltip(shortcut: &str) -> String {
    format!(
        "Valider les fichiers cochés (Ctrl+Entrée dans le message ; {shortcut} ouvre ce panneau)"
    )
}

fn operation_name(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Merge => "Fusion",
        OperationKind::Rebase => "Rebase",
        OperationKind::CherryPick => "Cherry-pick",
        OperationKind::Revert => "Revert",
    }
}

pub fn operation_conflicts(kind: OperationKind, n: usize) -> String {
    let files = if n > 1 { "fichiers" } else { "fichier" };
    format!("{} en cours — {n} {files} en conflit", operation_name(kind))
}

pub fn confirm_abort_title(kind: OperationKind) -> String {
    format!(
        "Abandonner l'opération en cours ({}) ?",
        operation_name(kind).to_lowercase()
    )
}

pub fn confirm_keep_title(side: ConflictSide, names: &[String]) -> String {
    let what = match side {
        ConflictSide::Ours => "Garder la nôtre",
        ConflictSide::Theirs => "Prendre la leur",
    };
    if names.len() == 1 {
        format!("{what} pour {} ?", names[0])
    } else {
        format!("{what} pour {} fichiers ?", names.len())
    }
}

pub fn confirm_keep_detail(side: ConflictSide, deleted: &[String]) -> String {
    let mut text = match side {
        ConflictSide::Ours => {
            "La copie de travail est remplacée par notre version, marqueurs de conflit compris."
                .to_owned()
        }
        ConflictSide::Theirs => {
            "La copie de travail est remplacée par leur version, marqueurs de conflit compris."
                .to_owned()
        }
    };
    if !deleted.is_empty() {
        text.push_str(&format!(
            "\nCe choix SUPPRIME du disque et de l'index : {}.",
            deleted.join(", ")
        ));
    }
    text
}

pub fn relative_age(seconds: u64) -> String {
    match seconds {
        0..=59 => "à l'instant".to_owned(),
        60..=3599 => format!("il y a {} min", seconds / 60),
        3600..=86_399 => format!("il y a {} h", seconds / 3600),
        _ => format!("il y a {} j", seconds / 86_400),
    }
}

// ----- Dialogue de fusion ---------------------------------------------------------------------------
pub const MERGE_BUTTON: &str = "Fusionner…";
pub const MERGE_DISABLED_DELETION: &str =
    "Un côté a supprimé ce fichier : choisissez de le garder ou de le supprimer.";
pub const MERGE_DISABLED_ONE_SIDE: &str =
    "Le fichier n'existe que d'un côté : choisissez de le garder ou de ne pas le prendre.";
pub const MERGE_TITLE_OURS: &str = "Notre version";
pub const MERGE_TITLE_RESULT: &str = "Résultat";
pub const MERGE_TITLE_THEIRS: &str = "Leur version";
pub const MERGE_SUBTITLE: &str = "Fusion à trois volets";
pub const MERGE_CLOSE: &str = "Fermer (Échap)";
pub const MERGE_ALL_DONE: &str = "Tout est traité";
pub const MERGE_PREVIOUS: &str = "Bloc en attente précédent (Maj+F7)";
pub const MERGE_NEXT: &str = "Bloc en attente suivant (F7)";
pub const MERGE_APPLY_LABEL: &str = "Appliquer les changements non conflictuels :";
pub const MERGE_APPLY_ALL: &str = "Tous";
pub const MERGE_APPLY_LEFT: &str = "De gauche";
pub const MERGE_APPLY_RIGHT: &str = "De droite";
pub const MERGE_APPLY_ALL_TIP: &str =
    "Applique les changements de gauche, de droite et identiques ; les conflits ne sont jamais touchés";
pub const MERGE_APPLY_LEFT_TIP: &str = "Applique les changements de gauche et les identiques";
pub const MERGE_APPLY_RIGHT_TIP: &str = "Applique les changements de droite et les identiques";
pub const MERGE_UNDO: &str = "Annuler (Ctrl+Z)";
pub const MERGE_REDO: &str = "Rétablir (Ctrl+Maj+Z)";
pub const MERGE_SAVE: &str = "Enregistrer le résultat";
pub const MERGE_SAVE_TIP: &str =
    "Enregistrer le résultat et marquer le fichier résolu (Ctrl+Entrée)";
pub const MERGE_SAVING: &str = "Enregistrement…";
pub const MERGE_CANCEL: &str = "Annuler";
pub const MERGE_CANCEL_TIP: &str = "Fermer sans rien écrire (Échap)";
pub const MERGE_FOOTER_HINT: &str =
    "F7 / Maj+F7 : bloc suivant / précédent    Alt+Gauche / Droite : accepter    Alt+Maj+Gauche / Droite : ignorer";
pub const MERGE_REBASE_HELP: &str =
    "Pendant un rebase, un cherry-pick ou un revert, Git appelle « nôtre » la branche sur laquelle on rejoue, et « leur » le commit rejoué.";
pub const MERGE_ASK_PENDING_TITLE: &str = "Il reste des blocs à traiter";
pub const MERGE_SAVE_ANYWAY: &str = "Enregistrer quand même";
pub const MERGE_BACK: &str = "Revenir";
pub const MERGE_ASK_DISCARD_TITLE: &str = "Abandonner la fusion de ce fichier ?";
pub const MERGE_ASK_DISCARD_DETAIL: &str =
    "Rien n'est écrit : le fichier reste en conflit, comme avant.";
pub const MERGE_DISCARD: &str = "Abandonner";
pub const MERGE_KEEP_WORKING: &str = "Continuer la fusion";
pub const MERGE_ASK_OVERWRITE_TITLE: &str = "Le fichier a changé sur le disque";
pub const MERGE_ASK_OVERWRITE_DETAIL: &str =
    "Il a été modifié depuis l'ouverture de la fusion. L'écraser avec le résultat ?";
pub const MERGE_OVERWRITE: &str = "Écraser";

pub fn merge_title(name: &str) -> String {
    format!("Fusion de {name}")
}

pub fn merge_pane_title(base: &str, branch: Option<&str>) -> String {
    match branch {
        Some(branch) => format!("{base} — {branch}"),
        None => base.to_owned(),
    }
}

pub fn merge_counts(changes: usize, conflicts: usize) -> String {
    let mut parts = Vec::new();
    if changes > 0 {
        parts.push(format!(
            "{changes} {}",
            if changes > 1 {
                "changements"
            } else {
                "changement"
            }
        ));
    }
    if conflicts > 0 {
        parts.push(format!(
            "{conflicts} {}",
            if conflicts > 1 { "conflits" } else { "conflit" }
        ));
    }
    let restants = if changes + conflicts > 1 {
        "restants"
    } else {
        "restant"
    };
    format!("{} {restants}", parts.join(" et "))
}

pub fn merge_ask_pending_detail(changes: usize, conflicts: usize) -> String {
    let counts = merge_counts(changes, conflicts)
        .replace(" restants", "")
        .replace(" restant", "");
    format!("Il reste {counts}. Les blocs non traités garderont le texte actuel du centre.")
}

pub fn merge_eol_banner(style: &str) -> String {
    format!(
        "Les fins de ligne diffèrent entre les versions ; le résultat utilisera celles de notre version ({style})."
    )
}

/// Bandeau du dialogue de fusion quand les versions ne sont pas d'accord sur la
/// marque d'ordre des octets (BOM). `with_bom` : le résultat en porte une.
/// `theirs_dropped` : leur version avait ajouté ou retiré la marque, et le résultat
/// ne suit pas ce changement.
pub fn merge_bom_banner(with_bom: bool, theirs_dropped: bool) -> String {
    let result = if with_bom {
        "le résultat garde la marque BOM de notre version"
    } else {
        "le résultat reste sans marque BOM, comme notre version"
    };
    if theirs_dropped {
        format!("Leur version change la marque BOM en tête de fichier ; {result}.")
    } else {
        format!("Les versions diffèrent sur la marque BOM en tête de fichier ; {result}.")
    }
}

pub fn merge_tip_accept(left: bool, shortcut: &str) -> String {
    if left {
        format!("Accepter la version de gauche ({shortcut})")
    } else {
        format!("Accepter la version de droite ({shortcut})")
    }
}

pub fn merge_tip_append(shortcut: &str) -> String {
    format!("Ajouter à la suite du texte déjà accepté ({shortcut})")
}

pub fn merge_tip_ignore(left: bool, shortcut: &str) -> String {
    if left {
        format!("Ignorer la version de gauche ({shortcut})")
    } else {
        format!("Ignorer la version de droite ({shortcut})")
    }
}

pub fn merge_refused(reason: &str, file: &str) -> String {
    format!(
        "Fusion de {file} impossible : {reason}. Choisissez un côté (Garder la nôtre / Prendre la leur)."
    )
}

/// Infobulle du bouton « Fusionner… » grisé d'après le contenu des versions.
pub fn merge_disabled_tip(reason: &str) -> String {
    format!(
        "Fusion à trois volets impossible : {reason}. Choisissez un côté (Garder la nôtre / Prendre la leur)."
    )
}

pub const MERGE_REASON_BINARY: &str = "fichier binaire";
pub const MERGE_REASON_NOT_UTF8: &str = "le fichier n'est pas de l'UTF-8";
pub const MERGE_REASON_TOO_LARGE: &str = "fichier trop gros (plus de 20 Mio ou 200 000 lignes)";
pub const MERGE_REASON_UNSUPPORTED: &str = "sous-module ou lien symbolique";
pub const MERGE_REASON_DELETION: &str = "un côté a supprimé le fichier";
pub const MERGE_REASON_ONE_SIDE: &str = "le fichier n'existe que d'un côté";

// ----- Tests ------------------------------------------------------------------------------------------
pub const TESTS_IDLE: &str = "Aucune exécution. Lancez la suite avec Ctrl+Maj+F10.";
pub const TESTS_DETECTING: &str = "Détection de PHP et de PHPUnit…";
pub const TESTS_ROOT: &str = "Résultats des tests";
pub const TESTS_RERUN: &str = "Relancer toute la suite";
pub const TESTS_SHOW_PASSED: &str = "Afficher les réussis";
pub const TESTS_EXPECTED: &str = "Attendu :";
pub const TESTS_ACTUAL: &str = "Obtenu :";
pub const TESTS_TRACE: &str = "Trace :";
pub const TESTS_OUTPUT: &str = "Sortie :";
pub const TESTS_NO_OUTPUT: &str = "Aucune sortie pour ce nœud.";
pub const TESTS_COMPARE: &str = "Comparer attendu / obtenu";
pub const TESTS_COMPARE_TIP: &str = "Ouvre les deux valeurs côte à côte, différences en couleur";
pub const TESTS_RESTART_TITLE: &str = "Une exécution est en cours";
pub const TESTS_RESTART_DETAIL: &str =
    "L'arrêter et relancer toute la suite ? Les résultats déjà obtenus seront remplacés.";
pub const TESTS_RESTART_YES: &str = "Arrêter et relancer";

pub fn tests_run_tip(shortcut: &str) -> String {
    format!("Lancer la suite PHPUnit ({shortcut})")
}

pub fn tests_stop_tip(shortcut: &str) -> String {
    format!("Arrêter l'exécution ({shortcut})")
}

pub fn tests_versions(php: Option<&str>, phpunit: Option<&str>) -> String {
    format!(
        "PHP {} — PHPUnit {}",
        php.unwrap_or("?"),
        phpunit.unwrap_or("?")
    )
}

pub fn tests_php_ok(path: &Path, version: Option<&str>) -> String {
    match version {
        Some(version) => format!("PHP {version} : {}", path.display()),
        None => format!("PHP : {}", path.display()),
    }
}

pub fn tests_phpunit_ok(path: &Path, version: Option<&str>) -> String {
    match version {
        Some(version) => format!("PHPUnit {version} : {}", path.display()),
        None => format!("PHPUnit : {}", path.display()),
    }
}

pub fn tests_config_ok(path: &Path) -> String {
    format!("Configuration : {}", path.display())
}

pub fn tests_duration(duration: Duration) -> String {
    format!("Durée : {}", crate::kit::format_duration(duration))
}

pub fn tests_process_exited(code: Option<i32>) -> String {
    match code {
        Some(code) => {
            format!("Le processus PHP s'est arrêté pendant ce test (code de sortie {code}).")
        }
        None => "Le processus PHP s'est arrêté pendant ce test.".to_owned(),
    }
}

pub fn tests_counts(counts: &Counts, elapsed: Duration) -> String {
    let total = counts.total.map_or("?".to_owned(), |n| n.to_string());
    let mut text = format!(
        "Réussis : {}, échoués : {}, en erreur : {}, ignorés : {} sur {total}",
        counts.passed,
        counts.failed,
        counts.errors,
        counts.ignored()
    );
    if counts.risky > 0 {
        text.push_str(&format!(", risqués : {}", counts.risky));
    }
    if counts.interrupted > 0 {
        text.push_str(&format!(", interrompus : {}", counts.interrupted));
    }
    text.push_str(&format!(" — {}", crate::kit::format_duration(elapsed)));
    text
}

pub fn tests_summary(summary: &RunSummary, elapsed: Duration) -> String {
    let counts = &summary.counts;
    let duration = crate::kit::format_duration(elapsed);
    match summary.outcome {
        RunOutcome::Completed if counts.started == 0 => "Aucun test exécuté".to_owned(),
        RunOutcome::Completed if counts.failures() == 0 => {
            let total = counts.total.unwrap_or(counts.finished());
            let mut text = format!("Tests réussis : {} sur {total} — {duration}", counts.passed);
            if counts.ignored() + counts.risky > 0 {
                text.push_str(&format!(
                    " (ignorés : {}, risqués : {})",
                    counts.ignored(),
                    counts.risky
                ));
            }
            if let Some(code) = summary.exit_code.filter(|c| *c != 0) {
                text.push_str(&format!(" — code de sortie {code}"));
            }
            text
        }
        RunOutcome::Completed => format!(
            "{} {}, {} {} sur {} — {duration}",
            counts.failed,
            if counts.failed > 1 {
                "échecs"
            } else {
                "échec"
            },
            counts.errors,
            if counts.errors > 1 {
                "erreurs"
            } else {
                "erreur"
            },
            counts.total.unwrap_or(counts.finished())
        ),
        RunOutcome::Aborted { exit_code } => match exit_code {
            Some(code) => format!("Exécution interrompue (code de sortie {code}) — {duration}"),
            None => format!("Exécution interrompue — {duration}"),
        },
        RunOutcome::Stopped => format!("Arrêté — {duration}"),
        RunOutcome::TimedOut => format!("Délai dépassé — {duration}"),
    }
}

pub fn test_state(state: TestState) -> &'static str {
    match state {
        TestState::Pending => "en attente",
        TestState::Running => "en cours",
        TestState::Passed => "réussi",
        TestState::Skipped => "ignoré",
        TestState::Incomplete => "incomplet",
        TestState::Risky => "risqué",
        TestState::Interrupted => "interrompu",
        TestState::Failed => "échoué",
        TestState::Error => "erreur",
    }
}

pub fn problem_text(problem: &Problem) -> String {
    let list = |paths: &[std::path::PathBuf]| {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    match problem {
        Problem::PhpNotFound { tried } => format!(
            "PHP introuvable. Indiquez son chemin dans Réglages, page PHPUnit, ou ajoutez-le au PATH. Cherché : {}",
            list(tried)
        ),
        Problem::PhpPathInvalid(path) => {
            format!("Le chemin de PHP indiqué dans les réglages n'existe pas : {}", path.display())
        }
        Problem::PhpNotRunnable { path, reason } => {
            format!("PHP ne s'exécute pas ({}) : {reason}", path.display())
        }
        Problem::PhpUnitNotFound { searched } => format!(
            "PHPUnit introuvable. Indiquez son script dans Réglages, page PHPUnit (cherché : {})",
            list(searched)
        ),
        Problem::PhpUnitPathInvalid(path) => {
            format!("Le chemin de PHPUnit indiqué dans les réglages n'existe pas : {}", path.display())
        }
        Problem::NoConfiguration { root } => {
            format!("Aucun phpunit.xml à la racine du projet ({}). Créez-le, ou indiquez le bon fichier dans Réglages, page PHPUnit.", root.display())
        }
        Problem::ConfigurationPathInvalid(path) => format!(
            "Le fichier de configuration PHPUnit indiqué dans les réglages n'existe pas : {}",
            path.display()
        ),
    }
}

// ===== Lot v1 : branches, distant, opérations Git, menus contextuels, réglages =====
// (anciennement texts_v1.rs ; relu par `redacteur`)
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
pub const REASON_NO_UPSTREAM: &str = "Cette branche n'a pas d'amont.";
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
pub const TIP_FETCH: &str =
    "Récupérer (git fetch --all --prune) : met à jour les distants et retire les branches qui n'existent plus.";
pub const UPSTREAM_GONE_MARK: &str = "amont supprimé";
pub const TREE_ALL: &str = "Toutes les branches";

// ----- Dialogues : titres, boutons, textes ------------------------------------------------
pub const DLG_NEW_BRANCH: &str = "Nouvelle branche";
pub const DLG_FORCE_SWITCH: &str = "Forcer la bascule ?";
pub const DLG_UPDATE_METHOD: &str = "Mettre à jour le projet";
pub const DLG_PUSH_REJECTED: &str = "Le distant a des commits que vous n'avez pas encore";

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
    "Branche protégée : le push forcé est interdit (réglage « Branches protégées », page Git).";

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
pub const SWITCH_OVERWRITE_STASH_NOTE: &str = "« Mettre de côté et basculer » met vos changements de côté (git stash), bascule, puis les remet. Si la remise entre en conflit, ils restent mis de côté (git stash list).";
pub const FORCE_SWITCH_WARNING: &str =
    "Les changements locaux de ces fichiers seront jetés de votre copie de travail.";
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
    format!("Récupérable : « Restaurer » juste après, ou avec git branch <nom> {short}.")
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
pub const NO_UPSTREAM_TITLE: &str = "Pas d'amont";
pub const UPSTREAM_GONE_TITLE: &str = "L'amont n'existe plus";

pub fn no_upstream_text(branch: &str) -> String {
    format!("{branch} n'a pas d'amont. Aucune fusion ni rebase n'a été lancé.")
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
    format!("Remise en conflit : « {message} » reste mis de côté (git stash list).")
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
        format!("Branche réinitialisée sur {short} ; vos changements sont mis de côté (git stash list).")
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
    format!(
        "Fermer {} avec des modifications non enregistrées ?",
        plural(n, "onglet", "onglets")
    )
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

// ===== Lot v1 bis : réglages, menus contextuels de l'éditeur, des tests, de la recherche, du terminal =====
// (écrit par dev-interface ; relu par `redacteur`)

// ----- Fenêtre de réglages ------------------------------------------------------------
pub const ACTION_SETTINGS: &str = "Réglages…";
pub const SETTINGS_TITLE: &str = "Réglages";
pub const SETTINGS_FILTER_HINT: &str = "Filtrer les réglages…";
pub const SETTINGS_NO_MATCH: &str = "Aucun réglage ne correspond.";
pub const SETTINGS_PAGE_PHPUNIT: &str = "PHPUnit";
pub const SETTINGS_PAGE_GIT: &str = "Git";
pub const SETTINGS_PAGE_TERMINAL: &str = "Terminal";
pub const SETTINGS_PAGE_SEARCH: &str = "Recherche";
pub const SETTINGS_PAGE_APPEARANCE: &str = "Apparence";
pub const SETTINGS_SCOPE_PROJECT: &str = "Ce projet";
pub const SETTINGS_SCOPE_USER: &str = "Tous mes projets";
pub const SETTINGS_SCOPE_LABEL: &str = "Portée :";
pub const SETTINGS_SCOPE_USER_ONLY: &str = "Ce réglage vaut pour tous les projets.";
pub const SETTINGS_NO_USER_FILE: &str =
    "Pas de dossier de configuration : les réglages de tous les projets sont indisponibles.";
pub const SETTINGS_NO_PROJECT: &str =
    "Aucun projet ouvert : seuls les réglages de tous mes projets existent.";
pub const SETTINGS_OPEN_FILE: &str = "Ouvrir le fichier de réglages";
pub const SETTINGS_OK: &str = "Enregistrer et fermer";
pub const SETTINGS_CANCEL: &str = "Annuler";
pub const SETTINGS_APPLY: &str = "Appliquer";
pub const SETTINGS_BROWSE: &str = "Parcourir…";
pub const SETTINGS_REVERT: &str = "Revenir à la valeur héritée";
pub const SETTINGS_INHERITED_CHOICE: &str = "Hérité";
pub const SETTINGS_MODIFIED_MARK: &str = "modifié";
pub const SETTINGS_ORIGIN_PROJECT: &str = "réglé pour ce projet";
pub const SETTINGS_ORIGIN_USER: &str = "réglé pour tous mes projets";
pub const SETTINGS_ORIGIN_DEFAULT: &str = "par défaut";
pub const SETTINGS_ORIGIN_USER_FROM_PROJECT: &str = "hérité : réglé pour tous mes projets";
pub const SETTINGS_PROJECT_OVERRIDES: &str = "ce projet utilise sa propre valeur";
pub const SETTINGS_EXCLUDED_ALWAYS: &str = ".git et .juliestorm sont toujours exclus.";
pub const SETTINGS_EXCLUDED_RULE: &str =
    "nom : à toute profondeur ; /nom : à la racine seulement. Un motif par ligne.";
pub const SETTINGS_LINES_HINT: &str = "Un par ligne.";
pub const SETTINGS_ARGS_HINT: &str = "Séparés par des espaces ; les guillemets doubles groupent.";
pub const SETTINGS_ENV_ADD: &str = "Ajouter une variable";
pub const SETTINGS_ENV_NAME_HINT: &str = "NOM";
pub const SETTINGS_ENV_VALUE_HINT: &str = "valeur";
pub const SETTINGS_ENV_REMOVE: &str = "Retirer cette variable";
pub const SETTINGS_ENV_EMPTY: &str = "Aucune variable réglée à cette portée.";
pub const SETTINGS_COMMAND_TITLE: &str = "Commande qui sera lancée";
pub const SETTINGS_PROBE_RUNNING: &str = "Vérification…";
pub const SETTINGS_BLOCKED_PREFIX: &str = "Impossible d'enregistrer : ";
pub const SETTINGS_WRITE_FAILED_PREFIX: &str = "Réglages non enregistrés : ";
pub const SETTINGS_UNREADABLE_OPEN: &str = "Ouvrir le fichier";
pub const SETTINGS_REMEMBER_SAVED_TOAST: &str =
    "Choix enregistré pour ce projet : « Méthode de mise à jour du projet » (page Git).";
pub const SETTINGS_REMEMBER_NOTE: &str =
    "Enregistré dans les réglages de ce projet : « Méthode de mise à jour du projet ».";

pub fn settings_label(key: &str) -> &'static str {
    match key {
        "phpunit.php_path" => "Interpréteur PHP",
        "phpunit.phpunit_path" => "Script PHPUnit",
        "phpunit.configuration" => "Fichier de configuration",
        "phpunit.extra_args" => "Arguments supplémentaires",
        "phpunit.env" => "Variables d'environnement",
        "vcs.git_path" => "Exécutable Git",
        "vcs.update_method" => "Méthode de mise à jour du projet",
        "vcs.protected_branches" => "Branches protégées",
        "terminal.default_profile" => "Terminal par défaut",
        "terminal.git_bash_path" => "Git Bash (bash.exe)",
        "terminal.cmd_path" => "cmd (cmd.exe)",
        "index.excluded" => "Dossiers et fichiers exclus",
        "index.libraries" => "Dossiers de bibliothèques",
        "app.theme" => "Thème",
        _ => "",
    }
}

pub fn settings_choice_label(key: &str, value: &str) -> &'static str {
    match (key, value) {
        ("vcs.update_method", "fusion") => "Fusion",
        ("vcs.update_method", "rebase") => "Rebase",
        ("terminal.default_profile", "git-bash") => "Git Bash",
        ("terminal.default_profile", "cmd") => "cmd",
        ("app.theme", "sombre") => "Sombre",
        ("app.theme", "clair") => "Clair",
        _ => "",
    }
}

pub fn settings_detected(path: &str) -> String {
    format!("détection automatique : {path}")
}
pub const SETTINGS_NOT_DETECTED: &str = "non détecté";
pub fn settings_default_path_in_path(name: &str) -> String {
    format!("par défaut : {name}, cherché dans le PATH")
}
pub fn settings_arguments_count(n: usize, shown: &str) -> String {
    match n {
        0 => "aucun argument".to_owned(),
        1 => format!("1 argument : {shown}"),
        n => format!("{n} arguments : {shown}"),
    }
}

pub const SETTINGS_ERR_QUOTE: &str = "Guillemet non fermé.";
pub const SETTINGS_ERR_RELATIVE_USER: &str =
    "Un chemin relatif n'a pas de sens pour tous les projets : donnez un chemin complet ou un nom d'exécutable seul.";
pub const SETTINGS_ERR_ENV_NAME: &str = "Nom de variable vide ou contenant « = ».";
pub const SETTINGS_ERR_ENV_DUP: &str = "Nom de variable en double.";
pub fn settings_reserved_arg(arg: &str) -> String {
    format!("JulieStorm passe déjà l'option {arg} : la répéter casserait la commande ou la lecture des résultats.")
}
pub const SETTINGS_EMPTY_ARG: &str = "Argument vide : PHPUnit le prendrait pour un chemin.";

/// Niveau (0 réussi, 1 avertissement) et texte d'une sonde PHP / PHPUnit / configuration.
pub fn probe_text(result: &julie_phpunit::ProbeResult) -> (u8, String) {
    use julie_phpunit::{ProbeFailure, ProbeKind, ProbeSuccess};
    match result {
        Ok(ProbeSuccess::Php { version, .. }) => (
            0,
            if version.starts_with("PHP") {
                version.clone()
            } else {
                format!("PHP {version}")
            },
        ),
        Ok(ProbeSuccess::PhpUnit { version, .. }) => (0, format!("PHPUnit {}", version.text)),
        Ok(ProbeSuccess::Configuration { .. }) => (0, "Configuration PHPUnit lisible".to_owned()),
        Err(ProbeFailure::NotFound(_)) => (1, "Ce chemin n'existe pas".to_owned()),
        Err(ProbeFailure::IsDirectory(_)) => (1, "C'est un dossier, pas un fichier".to_owned()),
        Err(ProbeFailure::TimedOut(_)) => (1, "Pas de réponse en 5 s".to_owned()),
        Err(ProbeFailure::NotThis { kind, first_line }) => {
            let what = match kind {
                ProbeKind::Php => "PHP",
                ProbeKind::PhpUnit => "PHPUnit",
                ProbeKind::Configuration => "une configuration PHPUnit",
            };
            (1, format!("Ce n'est pas {what} : {first_line}"))
        }
        Err(ProbeFailure::NotRunnable { reason, .. }) => (1, format!("Ne se lance pas : {reason}")),
        Err(ProbeFailure::Unreadable { reason, .. }) => (1, format!("Illisible : {reason}")),
        Err(ProbeFailure::PhpUnavailable) => (
            1,
            "PHP ne répond pas correctement : version de PHPUnit impossible à lire".to_owned(),
        ),
    }
}
pub fn probe_not_git(first_line: &str) -> String {
    format!("Ce n'est pas Git : {first_line}")
}
pub const PROBE_EXISTS: &str = "Fichier trouvé";
pub const PROBE_NOT_EXISTS: &str = "Ce chemin n'existe pas";
pub const PROBE_IS_DIR: &str = "C'est un dossier, pas un fichier";
pub const PROBE_NO_ANSWER: &str = "Pas de réponse en 5 s";

pub fn settings_unreadable(file: &str, line: Option<u32>, message: &str) -> String {
    match line {
        Some(line) => format!("Réglages illisibles : {file}, ligne {line} : {message}"),
        None => format!("Réglages illisibles : {file} : {message}"),
    }
}
pub fn settings_refused(file: &str) -> String {
    format!("Le fichier {file} est illisible : rien n'a été écrit. Corrigez-le d'abord.")
}

// ----- Menus contextuels : éditeur, arbre, tests, recherche, terminal ---------------------
pub const ACTION_EDIT_CUT: &str = "Couper";
pub const ACTION_EDIT_COPY: &str = "Copier";
pub const ACTION_EDIT_PASTE: &str = "Coller";
pub const ACTION_EDIT_SELECT_ALL: &str = "Tout sélectionner";
pub const ACTION_GOTO_DECLARATION: &str = "Aller à la déclaration";
pub const ACTION_FIND_USAGES: &str = "Trouver les usages";
pub const ACTION_RUN_TESTS_HERE: &str = "Lancer les tests";
pub const LABEL_RUN_TESTS_FILE: &str = "Lancer les tests de ce fichier";
pub const LABEL_RUN_TESTS_DIR: &str = "Lancer les tests de ce dossier";
pub const LABEL_RUN_TESTS_CONFIG: &str = "Lancer la suite de cette configuration";
pub const ACTION_TREE_NEW_FILE: &str = "Fichier…";
pub const ACTION_TREE_NEW_FOLDER: &str = "Dossier…";
pub const MENU_NEW: &str = "Nouveau";
pub const ACTION_TREE_RENAME: &str = "Renommer…";
pub const ACTION_TREE_DELETE: &str = "Supprimer…";
pub const ACTION_TEST_RUN_NODE: &str = "Lancer ce test";
pub const LABEL_RUN_TEST: &str = "Lancer ce test";
pub const LABEL_RUN_CLASS: &str = "Lancer cette classe";
pub const LABEL_RUN_SUITE: &str = "Lancer cette suite";
pub const LABEL_RUN_ALL: &str = "Relancer toute la suite";
pub const ACTION_TEST_RERUN_FAILED: &str = "Relancer les tests en échec";
pub const ACTION_TEST_GOTO: &str = "Aller au code source";
pub const ACTION_TEST_COPY_NAME: &str = "Copier le nom";
pub const ACTION_TEST_COMPARE: &str = "Comparer attendu et obtenu";
pub const ACTION_HIT_OPEN: &str = "Ouvrir à l'occurrence";
pub const ACTION_HIT_COPY_LINE: &str = "Copier la ligne";
pub const ACTION_HIT_EXCLUDE: &str = "Exclure ce fichier des résultats";
pub const ACTION_TERM_COPY: &str = "Copier";
pub const ACTION_TERM_PASTE: &str = "Coller";
pub const ACTION_TERM_SELECT_ALL: &str = "Tout sélectionner";
pub const ACTION_TERM_CLEAR: &str = "Effacer (écran et historique)";
pub const ACTION_TERM_RENAME: &str = "Renommer l'onglet…";
pub const ACTION_TERM_CLOSE: &str = "Fermer l'onglet";
pub const MENU_TERM_NEW: &str = "Nouveau terminal";
pub const ACTION_TERM_NEW_CMD: &str = "cmd";
pub const ACTION_TERM_NEW_BASH: &str = "Git Bash";

pub const REASON_READ_ONLY: &str = "Le document est en lecture seule.";
pub const REASON_NOT_A_TEST: &str =
    "Ce n'est ni un fichier de test (*Test.php) ni une configuration PHPUnit.";
pub const REASON_PHPUNIT_MISSING: &str =
    "PHPUnit introuvable : réglez-le dans Réglages, page PHPUnit.";
pub const REASON_NO_SOURCE: &str = "Ce nœud n'a pas de fichier source connu.";
pub const REASON_NO_FAILURE: &str = "Aucun test en échec dans la dernière exécution.";
pub const REASON_NO_COMPARISON: &str = "PHPUnit n'a pas fourni de comparaison pour ce test.";
pub const REASON_MULTI: &str = "Impossible sur plusieurs éléments à la fois.";
pub const REASON_PROJECT_ROOT: &str = "Impossible sur la racine du projet.";
pub const REASON_NO_TEXT_SELECTED: &str = "Aucun texte sélectionné.";
pub const REASON_NOT_SAVED: &str = "Ce fichier n'a jamais été enregistré.";
pub const REASON_NO_TESTS_RUN: &str = "Aucune exécution de tests dans cette session.";
pub const REASON_NOTHING_TO_DELETE: &str =
    "Seuls les fichiers non suivis peuvent être supprimés ici.";
pub const REASON_NO_TERMINAL: &str = "Aucun terminal ouvert.";
pub const REASON_NO_PROJECT_OPEN: &str = "Aucun projet ouvert.";

pub const DLG_NEW_FILE: &str = "Nouveau fichier";
pub const DLG_NEW_FOLDER: &str = "Nouveau dossier";
pub fn dlg_new_in(dir: &str) -> String {
    format!("Dans : {dir}")
}
pub const LABEL_NAME: &str = "Nom";
pub const HINT_NEW_NAME: &str = "nom.php ou dossier/nom.php";
pub fn dlg_rename_fs(name: &str) -> String {
    format!("Renommer « {name} »")
}
pub const BTN_RENAME_FS: &str = "Renommer";
pub fn dlg_delete(n: usize) -> String {
    if n == 1 {
        "Supprimer cet élément ?".to_owned()
    } else {
        format!("Supprimer ces {n} éléments ?")
    }
}
pub const DELETE_TRASH_NOTE: &str =
    "Ils iront dans la corbeille de Windows : vous pourrez les restaurer.";
pub fn delete_folder_files(name: &str, files: usize) -> String {
    format!("{name} — dossier, nombre de fichiers : {files}")
}
pub fn delete_unsaved(name: &str) -> String {
    format!("Les modifications non enregistrées de {name} resteront dans l'onglet (marqué « supprimé du disque »).")
}
pub const BTN_DELETE_TRASH: &str = "Mettre à la corbeille";
pub const DLG_DELETE_FOREVER: &str = "Suppression définitive";
pub const DELETE_FOREVER_TEXT: &str =
    "La corbeille n'est pas disponible pour ces éléments : ils seront supprimés définitivement. Cela ne pourra pas être annulé.";
pub const BTN_DELETE_FOREVER: &str = "Supprimer définitivement";
pub const NAME_EMPTY: &str = "Le nom est vide.";
pub const NAME_EXISTS: &str = "Un fichier ou un dossier de ce nom existe déjà.";
pub fn name_forbidden_char(c: char) -> String {
    format!("Caractère interdit par Windows : {c}")
}
pub const NAME_RESERVED: &str = "Nom réservé par Windows (CON, NUL, COM1…).";
pub const NAME_TRAILING: &str = "Un nom ne peut pas finir par un point ou par une espace.";
pub const NAME_SEPARATOR: &str = "Un nouveau nom ne contient pas de séparateur de dossier.";
pub fn fs_created(name: &str) -> String {
    format!("Créé : {name}")
}
pub fn fs_renamed(old: &str, new: &str) -> String {
    format!("Renommé : {old} → {new}")
}
pub fn fs_trashed(n: usize) -> String {
    if n == 1 {
        "1 élément mis à la corbeille.".to_owned()
    } else {
        format!("{n} éléments mis à la corbeille.")
    }
}
pub fn fs_deleted_forever(n: usize) -> String {
    format!("{n} élément(s) supprimé(s) définitivement.")
}
pub fn fs_failed(error: &str) -> String {
    format!("Opération impossible : {error}")
}
pub const CHECK_DISCARD_DELETE_ADDED: &str =
    "Supprimer aussi les fichiers ajoutés (vers la corbeille)";
pub fn trashed_added(n: usize) -> String {
    format!("{n} fichier(s) ajouté(s) mis à la corbeille.")
}
pub fn copied_line() -> &'static str {
    "Ligne copiée."
}
pub fn hit_excluded(name: &str) -> String {
    format!("{name} retiré des résultats de cette recherche.")
}
// ===== FIN DES TEXTES v1 bis =====

// ----- Navigation : aller à la déclaration, trouver les usages ------------------------------
pub const ACTION_NAV_BACK: &str = "Position précédente";
pub const ACTION_NAV_FORWARD: &str = "Position suivante";
pub const ACTION_USAGE_NEXT: &str = "Usage suivant";
pub const ACTION_USAGE_PREVIOUS: &str = "Usage précédent";
pub const ACTION_TOGGLE_USAGES: &str = "Usages";
pub const TOOL_USAGES: &str = "Usages";
pub const REASON_NO_CODE_FILE: &str = "Ouvrez un fichier PHP, JavaScript ou TypeScript.";
pub const NAV_DYNAMIC_NAME: &str = "Nom calculé à l'exécution.";
pub const NAV_NO_USAGES: &str = "Aucun usage trouvé.";
pub const NAV_LIST_FILTER_HINT: &str = "Tapez pour filtrer par nom ou par fichier";
pub const NAV_LIST_NO_MATCH: &str = "Aucune ligne ne correspond au filtre.";
pub const NAV_LIST_KEYS: &str = "Entrée : ouvrir   Échap : fermer";
pub const NAV_CONSTRUCTOR: &str = "constructeur";
pub const NAV_LIBRARY: &str = "bibliothèque";

pub fn nav_not_in_project(name: &str) -> String {
    format!(
        "Aucune déclaration dans le projet pour « {name} » (peut-être une fonction du langage)."
    )
}

pub fn nav_not_found(name: &str) -> String {
    format!("Aucune déclaration trouvée pour « {name} ».")
}

pub fn nav_external_module(specifier: &str) -> String {
    format!("Module externe (« {specifier} ») : rien à ouvrir dans le projet.")
}

pub fn nav_indexing(percent: u8) -> String {
    format!("Indexation en cours ({percent} %), réessayez dans un instant.")
}

pub fn symbol_kind_label(kind: julie_index::SymbolKind) -> &'static str {
    use julie_index::SymbolKind::*;
    match kind {
        Namespace => "espace de noms",
        Class => "classe",
        Interface => "interface",
        Trait => "trait",
        Enum => "énumération",
        EnumCase => "cas d'énumération",
        Function => "fonction",
        Method => "méthode",
        Property => "propriété",
        ClassConstant => "constante de classe",
        Constant => "constante",
        Variable => "variable",
        Parameter => "paramètre",
        Import => "import",
        TypeAlias => "alias de type",
    }
}

/// Titre de la liste surgissante des cibles.
pub fn nav_list_title(guess: Option<&julie_index::Guess>, name: &str) -> String {
    match guess {
        Some(julie_index::Guess::UnknownReceiver {
            receiver,
            member,
            kind,
        }) => {
            let what = match kind {
                julie_index::SymbolKind::Property => "propriétés",
                julie_index::SymbolKind::ClassConstant => "constantes",
                _ => "méthodes",
            };
            // L'index peut ne pas donner le texte du receveur (`$this->model`) : on ne l'invente pas.
            if receiver.trim().is_empty() {
                format!("Type de l'objet inconnu : {what} nommées {member}")
            } else {
                format!("Type de {receiver} inconnu : {what} nommées {member}")
            }
        }
        Some(julie_index::Guess::GlobalName { name }) => {
            format!("Nom global « {name} » : déclarations possibles (type inconnu)")
        }
        None => format!("Aller à « {name} »"),
    }
}

pub fn nav_usages_pick_title(name: &str) -> String {
    format!("Usages de quelle déclaration de « {name} » ?")
}

/// « 5 usages sûrs dans 3 fichiers, 1 appel possible » (le compte des appels possibles
/// est toujours dit, même à 0 ; jamais « tous »).
pub fn usages_headline(
    name: &str,
    sure: usize,
    files_sure: usize,
    possible: usize,
    running: bool,
) -> String {
    let mut text = format!(
        "{name} — {} dans {}, {}",
        plural(sure, "usage sûr", "usages sûrs"),
        plural(files_sure, "fichier", "fichiers"),
        plural(possible, "appel possible", "appels possibles"),
    );
    if running {
        text.push_str(" — recherche en cours…");
    }
    text
}

pub const USAGES_GROUP_SURE: &str = "Usages";
pub const USAGES_GROUP_POSSIBLE: &str = "Appels possibles (type inconnu)";
pub const USAGES_INCLUDE_LIBRARIES: &str = "Inclure les bibliothèques";
pub const USAGES_INCLUDE_TIP: &str =
    "Chercher aussi dans vendor et node_modules (plus lent) : relance la recherche.";
pub const USAGES_RELAUNCH: &str = "Relancer";
pub const USAGES_STOP: &str = "Arrêter";
pub const USAGES_CLOSE_TAB: &str = "Fermer cet onglet";
pub const USAGES_STOPPED: &str = "Recherche interrompue : résultats partiels.";
pub const USAGES_HELP_MEMBER: &str = "Les appels sur des valeurs de type inconnu sont listés à part. Les appels dynamiques ($obj->$nom(), call_user_func, noms dans des chaînes) ne sont pas cherchés.";
pub const USAGES_NONE_YET: &str = "Aucun usage pour l'instant.";
pub const USAGES_EMPTY: &str =
    "Aucune recherche d'usages : Alt+F7 sur un nom, ou Ctrl+clic sur sa déclaration.";
pub const USAGES_LIBRARY_TAG: &str = "bibliothèque";
pub const USAGES_UNSAVED_TAG: &str = "non enregistré";

pub fn usages_indexing_banner(percent: u8) -> String {
    format!("Indexation en cours ({percent} %) : résultats incomplets.")
}

pub const USAGES_INDEX_WAS_INCOMPLETE: &str =
    "L'indexation n'était pas finie au lancement : résultats incomplets, relancez la recherche.";

pub fn usages_files_read(n: usize) -> String {
    format!("{} lus", plural(n, "fichier", "fichiers"))
}

pub fn usages_libraries_skipped(n: usize) -> String {
    format!(
        "{} de bibliothèques non cherchés (case « Inclure les bibliothèques »)",
        plural(n, "fichier", "fichiers")
    )
}

pub fn usages_unreadable(n: usize) -> String {
    format!(
        "{} illisibles, binaires ou trop gros ignorés",
        plural(n, "fichier", "fichiers")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_agrees_in_number() {
        assert_eq!(find_summary(1, 1), "1 occurrence dans 1 fichier");
        assert_eq!(find_summary(12, 3), "12 occurrences dans 3 fichiers");
    }

    #[test]
    fn common_regex_errors_are_in_french() {
        for (pattern, expected) in [
            ("(", "groupe non fermé"),
            (")", "parenthèse fermante en trop"),
            ("[a", "classe de caractères non fermée"),
            ("*a", "répétition sans rien à répéter"),
            ("a{2", "répétition {n,m} non fermée"),
            (r"\q", "séquence d'échappement inconnue"),
            (r"a\", "échappement incomplet"),
            // Fin de motif sur une barre oblique inverse.
            ("(?<=a)b", "assertions avant et arrière"),
            (r"(a)\1", "références arrière"),
            ("[z-a]", "intervalle invalide"),
            ("a{3,1}", "intervalle de répétition invalide"),
            (r"\p{Nope}", "Unicode"),
        ] {
            let raw = regex::Regex::new(pattern).unwrap_err().to_string();
            let text = find_invalid_regex(&raw);
            assert!(text.contains(expected), "{pattern:?} : {raw:?} -> {text}");
            assert!(!text.contains("unclosed") && !text.contains("unopened"));
        }
        assert_eq!(regex_cause_fr("something new"), "syntaxe non reconnue");
    }
}
