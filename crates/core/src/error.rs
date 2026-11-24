//! Erreurs du noyau.
//!
//! Chaque crate métier définit sa propre énumération d'erreurs (avec `thiserror`)
//! et y enveloppe celle-ci par `#[from] julie_core::Error`. Le texte de `Display`
//! est destiné aux journaux et aux rapports de bogue : l'interface choisit son
//! message dans le catalogue de textes à partir de la variante.

use std::path::{Path, PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Entrée-sortie sur un fichier ou un dossier précis.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Le jeton d'annulation a été déclenché. Ce n'est pas un échec à montrer.
    #[error("opération annulée")]
    Cancelled,

    /// Le programme n'a pas pu être lancé (introuvable, droits).
    #[error("impossible de lancer « {program} » : {source}")]
    ProcessSpawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    /// Le programme a rendu un code de sortie d'échec
    /// (voir [`crate::process::Output::into_success`]).
    #[error("« {program} » a échoué (code {code:?}) : {stderr}")]
    ProcessFailed {
        program: String,
        code: Option<i32>,
        stderr: String,
    },

    /// Le texte contient un caractère absent de l'encodage du fichier : on refuse
    /// d'écrire plutôt que de le remplacer en silence.
    #[error("le caractère « {character} » (ligne {line}) n'existe pas en {encoding}")]
    Unencodable {
        character: char,
        /// Ligne du caractère, comptée à partir de zéro.
        line: u32,
        encoding: &'static str,
    },

    /// Fichier de réglages illisible ou section mal typée.
    #[error("réglages invalides ({origin}) : {message}")]
    Settings { origin: String, message: String },

    /// Donnée ou argument invalide, sans variante dédiée.
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Error::Cancelled)
    }
}
