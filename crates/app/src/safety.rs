//! Copie de sécurité avant d'écraser du travail (décision n° 3 de `decisions-v1.md`) :
//! « Annuler les modifications », la bascule forcée et la réinitialisation « tout jeter »
//! copient d'abord les fichiers touchés. La copie est faite par `julie_core::backup`
//! (manifeste, 7 jours) ; ce module n'est que la colle côté interface.

use std::path::{Path, PathBuf};

use julie_core::{backup, Project};
use julie_vcs::{OverwriteConflict, SwitchTarget};

/// Ce qui suit la copie.
#[derive(Debug, Clone)]
pub enum AfterCopy {
    Discard {
        paths: Vec<PathBuf>,
    },
    ForceSwitch {
        target: SwitchTarget,
        conflict: OverwriteConflict,
    },
    ResetHard {
        target: String,
        stash_first: bool,
    },
}

impl AfterCopy {
    /// Clé stable de l'opération, pour le manifeste.
    pub fn reason(&self) -> &'static str {
        match self {
            AfterCopy::Discard { .. } => "vcs.discard",
            AfterCopy::ForceSwitch { .. } => "vcs.checkout",
            AfterCopy::ResetHard { .. } => "vcs.reset",
        }
    }
}

/// Copie les fichiers (bloquant : dans un fil). Rend l'identifiant de la copie, ou
/// `None` s'il n'y avait rien à copier (tous supprimés). Une erreur interdit l'opération.
pub fn copy(root: &Path, reason: &str, files: &[PathBuf]) -> Result<Option<String>, String> {
    let existing: Vec<PathBuf> = files.iter().filter(|f| f.is_file()).cloned().collect();
    if existing.is_empty() {
        return Ok(None);
    }
    let project = Project::open(root).map_err(|e| e.to_string())?;
    backup::create(&project, reason, &existing)
        .map(|b| Some(b.id))
        .map_err(|e| e.to_string())
}

/// « Rétablir » : remet la copie. Rend le nombre de fichiers remis.
pub fn restore(root: &Path, id: &str) -> Result<usize, String> {
    let project = Project::open(root).map_err(|e| e.to_string())?;
    backup::restore(&project, id)
        .map(|r| r.restored.len())
        .map_err(|e| e.to_string())
}

/// Efface les copies de plus de 7 jours (à l'ouverture du projet, dans un fil).
pub fn purge(root: &Path) {
    if let Ok(project) = Project::open(root) {
        let _ = backup::purge_expired(&project);
    }
}
