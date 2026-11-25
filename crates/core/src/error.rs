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

    /// Fichier de réglages que l'on ne sait pas lire (syntaxe TOML, encodage) : on
    /// refuse d'y écrire tant qu'il est dans cet état. `line` à partir de un.
    #[error("réglages illisibles : {}{} : {message}", path.display(), line_suffix(*line))]
    SettingsUnreadable {
        path: PathBuf,
        line: Option<u32>,
        message: String,
    },

    /// La corbeille ne peut pas recevoir ces éléments (lecteur réseau, corbeille
    /// désactivée…). **Rien n'a été supprimé définitivement** : `remaining` liste
    /// ce qui est toujours sur le disque.
    #[error("corbeille indisponible pour {}", path_list(remaining))]
    TrashUnavailable { remaining: Vec<PathBuf> },

    /// La mise à la corbeille a échoué (fichier verrouillé par un autre programme,
    /// droits…). `remaining` : ce qui est toujours sur le disque ; `code` : le
    /// code d'erreur Windows (HRESULT), pour les journaux.
    #[error(
        "mise à la corbeille impossible (code {code:#010X}) pour {}",
        path_list(remaining)
    )]
    TrashFailed { remaining: Vec<PathBuf>, code: i32 },

    /// Donnée ou argument invalide, sans variante dédiée.
    #[error("{0}")]
    Invalid(String),
}

fn line_suffix(line: Option<u32>) -> String {
    line.map(|line| format!(", ligne {line}"))
        .unwrap_or_default()
}

/// Le premier chemin, et combien d'autres.
fn path_list(paths: &[PathBuf]) -> String {
    match paths {
        [] => "aucun élément".to_owned(),
        [only] => only.display().to_string(),
        [first, rest @ ..] => format!("{} et {} autre(s)", first.display(), rest.len()),
    }
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
