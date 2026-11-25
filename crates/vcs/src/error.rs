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

    /// Nom de branche refusé (voir [`BranchNameProblem`]).
    #[error("nom de branche refusé : {0:?}")]
    InvalidBranchName(BranchNameProblem),

    /// La branche n'existe pas.
    #[error("branche introuvable : {0}")]
    BranchNotFound(String),

    /// Geste impossible sur la branche courante (la supprimer…).
    #[error("c'est la branche courante : {0}")]
    CurrentBranch(String),

    /// `git branch -d` refuse : la branche porte des commits fusionnés ni dans
    /// la branche courante ni dans son amont. L'appelant confirme, puis appelle
    /// [`crate::force_delete_branch`].
    #[error("branche non fusionnée : {branch} ({commits} commit(s))")]
    BranchNotMerged {
        branch: String,
        /// Sommet de la branche (pour la recréer).
        tip: String,
        /// Nombre de commits qui ne seraient plus sur aucune branche.
        commits: u32,
        /// Sujets des 5 premiers.
        subjects: Vec<String>,
    },

    /// Branche protégée (réglage `[vcs] protected_branches`) : force push ou
    /// suppression distante refusés.
    #[error("branche protégée : {0}")]
    ProtectedBranch(String),

    /// Git refuse parce que des changements locaux seraient écrasés (bascule,
    /// fusion, rebase…). Rien n'a changé. L'appelant propose de mettre de côté
    /// ([`crate::LocalChanges::Stash`]) ou de forcer
    /// ([`crate::switch_discarding_changes`]).
    #[error("des changements locaux seraient écrasés : {0:?}")]
    WouldOverwrite(OverwriteConflict),

    /// Git demande une authentification qu'il ne peut pas obtenir ici
    /// (`GIT_TERMINAL_PROMPT=0`) ou l'a refusée. L'interface renvoie au terminal
    /// intégré ou au gestionnaire d'identifiants ; l'IDE ne demande jamais de
    /// mot de passe.
    #[error("authentification Git impossible : {stderr}")]
    AuthenticationFailed { stderr: String },

    /// La branche courante n'a pas d'amont (« Choisir l'amont… »).
    #[error("la branche {branch} n'a pas de branche distante associée")]
    NoUpstream { branch: String },

    /// L'amont a été supprimé sur le distant (`[gone]`).
    #[error("la branche distante {upstream} n'existe plus")]
    UpstreamGone { branch: String, upstream: String },

    /// Mise à jour d'une branche non courante refusée : elle a divergé de son
    /// amont (pas d'avance rapide possible sans basculer).
    #[error("la branche {branch} a divergé de son amont")]
    Diverged { branch: String },

    /// HEAD détachée : pas de branche à pousser, mettre à jour…
    #[error("HEAD détachée")]
    DetachedHead,

    /// Le push a été refusé (voir [`PushRejection`]). Rien n'a changé sur le
    /// distant pour cette référence.
    #[error("push refusé : {rejection:?}")]
    PushRejected {
        rejection: PushRejection,
        /// Sortie complète de Git (crochets et messages du serveur compris).
        output: String,
    },

    /// Commit de fusion : cherry-pick et revert ne sont pas pris en charge.
    #[error("commit de fusion non pris en charge : {0}")]
    MergeCommitNotSupported(String),

    /// Fichier déjà suivi par Git : l'ignorer ne le retire pas.
    #[error("fichier déjà suivi : {}", .0.display())]
    AlreadyTracked(PathBuf),

    /// `git add` refuse des chemins ignorés par `.gitignore`.
    #[error("chemins ignorés par .gitignore : {0:?}")]
    IgnoredPaths(Vec<PathBuf>),

    /// Le fichier n'est pas suivi par Git (`git mv` impossible : renommage
    /// simple, à faire par l'appelant).
    #[error("fichier non suivi : {}", .0.display())]
    NotTracked(PathBuf),

    /// La destination existe déjà : rien n'est écrasé.
    #[error("la destination existe déjà : {}", .0.display())]
    DestinationExists(PathBuf),

    /// Fichier en conflit : il faut d'abord le résoudre.
    #[error("fichier en conflit : {}", .0.display())]
    Conflicted(PathBuf),

    /// Demande mal formée (filtre `--follow` sur plusieurs chemins…).
    #[error("demande invalide : {0}")]
    InvalidRequest(String),
}

/// Pourquoi un nom de branche est refusé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchNameProblem {
    /// Vide ou seulement des espaces.
    Empty,
    /// Refusé par `git check-ref-format --branch` (espaces, `..`, `~ ^ : ? * [ \`,
    /// fin en `.lock` ou `/`…). `suggestion` : le nom avec ses espaces remplacés
    /// par des tirets, s'il est valable.
    Invalid { suggestion: Option<String> },
    /// Une branche locale porte déjà ce nom.
    AlreadyExists(String),
    /// Une branche porte ce nom à la casse près (Windows ne les distingue pas).
    SameNameDifferentCase(String),
    /// Le nom serait le dossier d'une branche existante (`feature` quand
    /// `feature/login` existe).
    FolderOfExisting(String),
    /// Une branche existante serait un dossier de ce nom (`feature/login/x`
    /// quand `feature/login` existe).
    InsideExisting(String),
    /// Un dossier du nom existe déjà avec une autre casse (`Feature/x` quand
    /// `feature/login` existe).
    FolderCaseMismatch(String),
}

/// Fichiers qui seraient écrasés, tels que Git les nomme.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverwriteConflict {
    /// Fichiers suivis modifiés localement.
    pub tracked: Vec<PathBuf>,
    /// Fichiers non suivis qu'un fichier suivi de la cible remplacerait.
    pub untracked: Vec<PathBuf>,
    /// Sortie de Git.
    pub output: String,
}

impl OverwriteConflict {
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.tracked.iter().chain(&self.untracked)
    }
}

/// Raison d'un refus de push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushRejection {
    /// Le distant a des commits absents de la branche : récupérer avant de
    /// pousser (fusionner ou rebaser). `known_behind` : commits du distant
    /// connus localement (d'après le dernier fetch ; 0 si on ne les a pas).
    NonFastForward { known_behind: u32 },
    /// `--force-with-lease` : le distant a bougé depuis ce que l'on en
    /// savait ; rien n'a été écrasé.
    StaleLease,
    /// Refus du serveur (crochet `pre-receive`, branche protégée côté
    /// serveur…) : `reason` est le texte de Git.
    RemoteRejected { reason: String },
    /// Autre refus (`reason` : texte de Git).
    Other { reason: String },
}

impl VcsError {
    /// Annulation demandée : ce n'est pas un échec à montrer.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, VcsError::Core(error) if error.is_cancelled())
    }
}
