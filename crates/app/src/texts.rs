//! Catalogue des textes de l'interface (français). **Provisoire** : `redacteur`
//! le relit et le complète. Aucun composant n'écrit de texte visible en dur ; tout
//! passe par ce module, regroupé par écran. Les textes qui portent des valeurs
//! sont des fonctions.
//!
//! (Les textes du terminal et de l'éditeur vivent dans leurs modules `texts`, qui
//! seront rapatriés ici avec le catalogue.)

use std::path::Path;

pub const APP_NAME: &str = "JulieStorm";

// ----- Menus et actions ----------------------------------------------------------
pub const MENU_FILE: &str = "Fichier";
pub const MENU_NAVIGATE: &str = "Aller à";
pub const MENU_VIEW: &str = "Affichage";

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
pub const TOOL_GIT_EMPTY: &str = "Changements locaux, journal et branches : à venir.";
pub const TOOL_TESTS_EMPTY: &str = "Aucun test lancé.";

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
        "vcs.diff" => "Calcul du diff",
        "vcs.commit" => "Commit",
        "vcs.log" => "Lecture du journal Git",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_agrees_in_number() {
        assert_eq!(find_summary(1, 1), "1 occurrence dans 1 fichier");
        assert_eq!(find_summary(12, 3), "12 occurrences dans 3 fichiers");
    }
}
