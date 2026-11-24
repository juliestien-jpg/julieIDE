//! Erreurs de `julie-text`.

use std::path::PathBuf;

/// Ce qui peut empêcher d'ouvrir ou d'enregistrer un document.
#[derive(Debug, thiserror::Error)]
pub enum TextError {
    /// Lecture, écriture ou encodage (voir `julie_core::Error`, dont
    /// `Unencodable` quand un caractère n'existe pas en Windows-1252).
    #[error(transparent)]
    Core(#[from] julie_core::Error),
    /// Octet NUL dans les 8 premiers Kio : le fichier n'est pas ouvert.
    #[error("fichier binaire : {path}")]
    Binary { path: PathBuf },
    /// Au-delà de [`crate::MAX_OPEN_SIZE`] : le fichier n'est pas ouvert.
    #[error("fichier trop gros ({size} octets) : {path}")]
    TooLarge { path: PathBuf, size: u64 },
    /// Le document n'a pas de chemin (nouveau fichier jamais enregistré).
    #[error("document sans chemin")]
    NoPath,
}

pub type TextResult<T> = std::result::Result<T, TextError>;
