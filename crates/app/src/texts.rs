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
use julie_vcs::{ConflictKind, ConflictSide, FileKind, Head, OperationKind, Upstream};

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
pub const ACTION_RELOAD_DISK: &str = "Relire le disque";
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
pub const ACTION_TOGGLE_THEME: &str = "Basculer thème sombre / clair";
pub const ACTION_TOGGLE_COMMIT: &str = "Panneau de validation";
pub const ACTION_BOTTOM_TALL: &str = "Agrandir / réduire la fenêtre du bas";
pub const ACTION_GIT_COMMIT: &str = "Valider les changements…";
pub const ACTION_GIT_REFRESH: &str = "Actualiser l'état Git";
pub const ACTION_TESTS_RUN: &str = "Lancer les tests";
pub const ACTION_TESTS_STOP: &str = "Arrêter les tests";
pub const ACTION_TESTS_RERUN: &str = "Relancer les tests";

// ----- Accueil, projet, arbre -------------------------------------------------------
pub const WELCOME_NO_PROJECT: &str = "Aucun projet ouvert.";
pub const PICK_FOLDER_TITLE: &str = "Ouvrir un dossier";
pub const TREE_NO_PROJECT: &str = "Aucun projet ouvert.";
pub const TERMINAL_NO_PROJECT: &str = "Ouvrez un projet pour lancer un terminal dans son dossier.";

pub fn project_open_failed(path: &Path, error: &str) -> String {
    format!(
        "Impossible d'ouvrir le dossier {} : {error}",
        path.display()
    )
}

pub fn settings_failed(error: &str) -> String {
    format!("Réglages illisibles, valeurs par défaut utilisées : {error}")
}

// ----- Onglets, éditeur ---------------------------------------------------------------
pub const UNTITLED: &str = "Sans titre";
pub const BANNER_CHANGED: &str = "Le fichier a changé sur le disque.";
pub const BANNER_DELETED: &str = "Supprimé du disque. Enregistrer le recrée.";
pub const BANNER_RELOAD: &str = "Recharger";
pub const BANNER_RELOAD_CONFIRM: &str = "Perdre mes modifications ?";
pub const BANNER_KEEP: &str = "Garder les miennes";
pub const BANNER_DISMISS: &str = "Fermer ce message";
pub const EOL_MIXED: &str = "mixte";
pub const ERR_BINARY: &str = "Fichier binaire";
pub const ERR_TOO_LARGE: &str = "Fichier trop gros";

pub fn load_failed(path: &Path, reason: &str) -> String {
    format!("{} : {reason}", path.display())
}

pub fn save_failed(path: &Path, reason: &str) -> String {
    format!("Enregistrement impossible, {} : {reason}", path.display())
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
pub const DIFF_REV_WORKTREE: &str = "Fichier de travail";
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
            None => text.push_str("  (amont disparu)"),
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
        OperationKind::Revert => "Annulation de commit (revert)",
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
            "Le fichier de travail est remplacé par notre version, marqueurs de conflit compris."
                .to_owned()
        }
        ConflictSide::Theirs => {
            "Le fichier de travail est remplacé par leur version, marqueurs de conflit compris."
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
pub const MERGE_SAVE: &str = "Appliquer";
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
pub const TESTS_RESTART_DETAIL: &str = "L'arrêter et relancer toute la suite ?";
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
        "{} réussis, {} échoués, {} en erreur, {} ignorés sur {total}",
        counts.passed,
        counts.failed,
        counts.errors,
        counts.ignored()
    );
    if counts.risky > 0 {
        text.push_str(&format!(", {} risqués", counts.risky));
    }
    if counts.interrupted > 0 {
        text.push_str(&format!(", {} interrompus", counts.interrupted));
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
                    " ({} ignorés, {} risqués)",
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
            "PHP introuvable. Indiquez son chemin dans les réglages ([phpunit] php_path) ou ajoutez-le au PATH. Cherché : {}",
            list(tried)
        ),
        Problem::PhpPathInvalid(path) => {
            format!("Le chemin de PHP réglé n'existe pas : {}", path.display())
        }
        Problem::PhpNotRunnable { path, reason } => {
            format!("PHP ne s'exécute pas ({}) : {reason}", path.display())
        }
        Problem::PhpUnitNotFound { .. } => "PHPUnit introuvable dans vendor/bin".to_owned(),
        Problem::PhpUnitPathInvalid(path) => {
            format!("Le chemin de PHPUnit réglé n'existe pas : {}", path.display())
        }
        Problem::NoConfiguration { root } => {
            format!("Aucun phpunit.xml à la racine du projet ({})", root.display())
        }
        Problem::ConfigurationPathInvalid(path) => format!(
            "Le fichier de configuration PHPUnit réglé n'existe pas : {}",
            path.display()
        ),
    }
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
