//! Erreurs de la crate.

use std::path::PathBuf;

/// Ce qui peut empêcher une session de démarrer ou de fonctionner.
///
/// Le texte de `Display` sert aux journaux ; ce que lit l'utilisatrice vient du
/// catalogue de textes, choisi par `app` d'après la variante.
#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error(transparent)]
    Core(#[from] julie_core::Error),
    /// L'interpréteur demandé n'existe pas à cet endroit.
    #[error("interpréteur introuvable : {0}")]
    ShellNotFound(PathBuf),
    /// Le dossier de départ n'existe pas.
    #[error("dossier de départ introuvable : {0}")]
    DirectoryNotFound(PathBuf),
    /// Un appel système a échoué (`call` : nom de la fonction Win32, `code` :
    /// `GetLastError` ou `HRESULT`).
    #[error("{call} a échoué (code {code:#x})")]
    Os { call: &'static str, code: u32 },
    /// Plate-forme sans pseudo-console (hors Windows).
    #[error("pseudo-console non disponible sur cette plate-forme")]
    Unsupported,
}
