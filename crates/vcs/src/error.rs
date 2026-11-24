//! Erreurs de `julie-vcs`.
//!
//! Le texte de `Display` sert aux journaux ; l'interface choisit son message dans
//! le catalogue de textes d'après la variante.

use std::path::PathBuf;

use crate::repo::OperationKind;

pub type Result<T, E = VcsError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    /// Erreur du noyau (disque, annulation, lancement de processus…).
    #[error(transparent)]
    Core(#[from] julie_core::Error),

    /// L'exécutable `git` est introuvable (réglage `[vcs] git_path`).
    #[error("git introuvable : « {program} »")]
    GitNotFound { program: String },

    /// Le dossier n'est dans aucun dépôt Git (ou seulement dans un `.git`).
    #[error("pas un dépôt Git : {}", .0.display())]
    NotARepository(PathBuf),

    /// Git refuse le dépôt : propriétaire différent (`safe.directory`).
    #[error("dépôt refusé par git (propriétaire différent) : {0}")]
    UnsafeRepository(String),

    /// Une autre opération Git tient le verrou `index.lock`.
    #[error("une autre opération Git est en cours : {0}")]
    IndexLocked(String),

    /// Git a rendu un code d'échec. `stdout` et `stderr` sont la sortie complète
    /// (crochets compris), à montrer telle quelle.
    #[error("git {command} a échoué (code {code:?}) : {stderr}")]
    GitFailed {
        /// Sous-commande et arguments, pour le journal.
        command: String,
        code: Option<i32>,
        stdout: String,
        stderr: String,
    },

    /// Sortie de Git qu'on ne sait pas lire : on refuse plutôt que deviner.
    #[error("sortie de git inattendue : {0}")]
    UnexpectedOutput(String),

    /// Le chemin n'est pas dans l'arbre de travail du dépôt.
    #[error("chemin hors du dépôt : {}", .0.display())]
    OutsideRepository(PathBuf),

    /// Révision refusée (vide, commence par `-`…).
    #[error("révision invalide : {0}")]
    InvalidRevision(String),

    /// Message de commit vide ou seulement des espaces.
    #[error("message de commit vide")]
    EmptyMessage,

    /// Commit sans fichier sélectionné (hors `amend`).
    #[error("aucun fichier sélectionné")]
    NothingSelected,

    /// Amender alors que le dépôt n'a pas encore de commit.
    #[error("le dépôt n'a pas encore de commit")]
    NoCommits,

    /// Commit de fichiers choisis refusé pendant une fusion, un rebase… :
    /// il faut conclure l'opération ([`crate::continue_operation`]).
    #[error("opération en cours : {0:?}")]
    OperationInProgress(OperationKind),

    /// Poursuivre ou abandonner alors qu'aucune opération n'est en cours.
    #[error("aucune opération en cours")]
    NoOperationInProgress,

    /// Il reste des fichiers en conflit.
    #[error("{} fichier(s) encore en conflit", .0.len())]
    UnresolvedConflicts(Vec<PathBuf>),

    /// Le fichier n'est pas (ou plus) en conflit.
    #[error("pas en conflit : {}", .0.display())]
    NotConflicted(PathBuf),

    /// Le fichier de travail a changé depuis qu'on l'a lu : on n'écrase pas.
    #[error("le fichier a changé sur le disque : {}", .0.display())]
    ChangedOnDisk(PathBuf),
}

impl VcsError {
    /// Annulation demandée : ce n'est pas un échec à montrer.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, VcsError::Core(error) if error.is_cancelled())
    }
}
