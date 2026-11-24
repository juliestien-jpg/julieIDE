//! Erreurs de `julie-index`.

use std::path::PathBuf;

pub type Result<T, E = IndexError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// Erreur du noyau (entrée-sortie, annulation…).
    #[error(transparent)]
    Core(#[from] julie_core::Error),

    /// Expression régulière invalide. `message` est celui du moteur `regex`
    /// (anglais, pour le journal) ; l'interface affiche son propre libellé sous
    /// le champ.
    #[error("expression invalide : {message}")]
    InvalidPattern { message: String },

    /// Un des masques du filtre de fichiers est invalide.
    #[error("masque de fichiers invalide « {mask} » : {message}")]
    InvalidMask { mask: String, message: String },

    /// Le dossier de la portée n'est pas dans le projet.
    #[error("dossier hors du projet : {}", .0.display())]
    OutsideProject(PathBuf),
}

impl IndexError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, IndexError::Core(error) if error.is_cancelled())
    }
}
