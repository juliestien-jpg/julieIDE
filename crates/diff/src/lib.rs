//! Diff, fusion à trois volets et patchs. Bibliothèque pure, sans Git ni fichiers.
//! Propriétaire : dev-fusion-diff.
//!
//! Cette crate ne dépend que de `julie-core` : ni egui, ni autre crate métier.
//! Elle reçoit des textes (`&str`), elle rend des structures. Tout est synchrone ;
//! les deux calculs longs ([`diff_lines`], [`MergeSession::new`]) prennent un
//! jeton d'annulation et se lancent dans une tâche de fond pour un gros fichier.
//!
//! # Conventions
//!
//! - **Lignes** : numérotées à partir de zéro selon le découpage de
//!   [`TextLines`] — une ligne finit par `\n` ou `\r\n` ; un `\r` isolé reste dans
//!   la ligne ; un texte vide a zéro ligne. Les plages sont `début..fin`, fin
//!   exclue ; une plage vide désigne un point d'insertion.
//! - **Entrées** : le contenu exact des fichiers, fins de ligne comprises (pas le
//!   texte normalisé en `\n` de `julie_core::file::read_text` pour un fichier
//!   CRLF : sinon le résultat de la fusion perd les fins de ligne d'origine).
//!   Une marque BOM en tête est acceptée, ignorée dans les comparaisons, et
//!   remise en tête du résultat de la fusion si notre version en avait une.
//! - **Rien n'est jamais « réparé »** : ce qui n'est pas modifié ressort à
//!   l'octet près.
//!
//! # Diff à deux côtés
//!
//! ```
//! use julie_diff::{diff_lines, DiffOptions, HunkKind};
//!
//! let cancel = julie_core::CancelToken::new();
//! let diff = diff_lines("a\nb\nc\n", "a\nB\nc\nd\n", &DiffOptions::default(), &cancel).unwrap();
//! assert_eq!(diff.hunks.len(), 2);
//! assert_eq!(diff.hunks[0].kind, HunkKind::Modified);   // b -> B
//! assert_eq!(diff.hunks[1].kind, HunkKind::Added);      // + d
//! // Connecteurs et défilement synchronisé : la correspondance complète.
//! for range in diff.alignment() {
//!     let _ = (range.left, range.right, range.hunk);
//! }
//! ```
//!
//! - [`Diff::hunks`] : blocs typés ([`HunkKind`]), avec l'affinage au mot et au
//!   caractère ([`InlineDiff`]) dans les blocs modifiés.
//! - [`Diff::alignment`], [`Diff::left_to_right`], [`Diff::right_to_left`] :
//!   correspondance des lignes.
//! - Par défaut, une différence de style de fins de ligne ou de saut de ligne
//!   final n'est **pas** un bloc : elle est rapportée à part
//!   ([`Diff::only_line_endings_differ`], [`Diff::final_newline`]).
//!   [`DiffOptions::exact`] compare à l'octet près.
//!
//! # Fusion à trois volets
//!
//! Spécification : `docs/specs/fusion.md`. [`merge_chunks`] rend le découpage
//! classé ; [`MergeSession`] est le modèle que le dialogue pilote :
//!
//! ```
//! use julie_diff::{ApplyScope, BlockKind, MergeSession, Side};
//!
//! let cancel = julie_core::CancelToken::new();
//! // base, notre version (gauche), leur version (droite)
//! let mut session = MergeSession::new(
//!     "a\nb\nc\nd\ne\n",
//!     "a\nB\nc\nd\nE1\n",
//!     "a\nb\nc\nd\nE2\n",
//!     &cancel,
//! ).unwrap();
//!
//! let blocks = session.blocks();
//! assert_eq!(blocks[0].kind, BlockKind::Left);
//! assert_eq!(blocks[1].kind, BlockKind::Conflict);
//! assert_eq!((session.counts().changes, session.counts().conflicts), (1, 1));
//!
//! session.apply_non_conflicting(ApplyScope::All);      // n'applique que b -> B
//! session.accept(1, Side::Left).unwrap();               // résout le conflit
//! session.accept(1, Side::Right).unwrap();              // « ajouter à la suite »
//! assert!(session.is_all_resolved());
//! assert_eq!(session.result_text(), "a\nB\nc\nd\nE1\nE2\n");
//!
//! session.undo();                                        // défait l'ajout à la suite
//! assert_eq!(session.result_text(), "a\nB\nc\nd\nE1\n");
//! ```
//!
//! Ce que l'interface lit à chaque image : [`MergeSession::blocks`] (nature,
//! état de chaque côté, place actuelle dans le résultat),
//! [`MergeSession::alignment`] (correspondance des trois volets),
//! [`MergeSession::result_lines`], [`MergeSession::counts`],
//! [`MergeSession::line_endings`] (bandeau). À l'enregistrement :
//! [`MergeSession::result_text`].
//!
//! **Décision sur la question ouverte de la spécification** — accepter les deux
//! côtés d'un conflit : le texte suit **l'ordre des clics**. Le premier côté
//! accepté remplace le texte du bloc ; le second est ajouté à la suite, à la fin
//! du bloc (donc après ce qui aurait été tapé entre-temps dans le bloc).
//! [`MergeSession::accept_both`] fait les deux en une étape annulable.
//!
//! **Plus prudent que Git, exprès** — un changement dont la place est ambiguë
//! (une ligne vide retirée d'une série, une fonction insérée entre deux
//! accolades, une ligne déplacée) est testé sur toute sa zone d'ambiguïté : nous
//! déclarons parfois un conflit là où `git merge-file` fusionne, jamais
//! l'inverse sur le corpus (`tests/fixtures/diff/merge/cases.txt`, chaque écart y
//! est expliqué) ni sur les tirages aléatoires de `tests/git_compare.rs`. Seul
//! écart dans l'autre sens, voulu par la spécification : une conversion de fins
//! de ligne n'est pas un changement (cas 15), alors que Git compare les octets.
//!
//! La crate ne refuse aucun texte : les limites de taille, le binaire et le
//! non-UTF-8 se décident chez l'appelant.
//!
//! # Prévu pour la v1 (emplacements réservés, rien de codé)
//!
//! - [`resolve`] : résolution automatique des conflits simples, une règle par
//!   sous-module.
//! - [`patch`] : écriture, lecture et application de diffs unifiés.

mod diff;
mod inline;
mod merge;
mod myers;
pub mod patch;
pub mod resolve;
mod session;
mod text;

pub use diff::{
    diff_lines, AlignedRange, Diff, DiffOptions, FinalNewlineChange, Hunk, HunkKind, LineMatch,
    Whitespace,
};
pub use inline::{InlineDiff, InlineSpan};
pub use merge::{merge_chunks, Chunk, ChunkKind};
pub use session::{
    AlignedSegment, ApplyScope, Block, BlockKind, LineEndingReport, MergeCounts, MergeSession,
    SessionError, Side, SideState,
};
pub use text::{LineEnding, LineEndingStyle, TextLines, BOM};

/// Erreur d'un calcul de cette crate. En pratique : l'annulation
/// (`julie_core::Error::Cancelled`), qui n'est pas un échec.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error(transparent)]
    Core(#[from] julie_core::Error),
}

impl DiffError {
    pub fn is_cancelled(&self) -> bool {
        match self {
            DiffError::Core(error) => error.is_cancelled(),
        }
    }
}

/// Résultat d'une fusion faite d'un trait par [`merge`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Le texte fusionné. Les conflits y gardent le texte de la base.
    pub text: String,
    /// Nombre de conflits laissés en attente.
    pub conflicts: usize,
}

/// Fusion d'un trait : ouvre une session, applique tous les changements non
/// conflictuels et rend le résultat. Pour les tests, les comparaisons et les
/// appelants sans interface.
pub fn merge(
    base: &str,
    ours: &str,
    theirs: &str,
    cancel: &julie_core::CancelToken,
) -> Result<MergeOutcome, DiffError> {
    let mut session = MergeSession::new(base, ours, theirs, cancel)?;
    session.apply_non_conflicting(ApplyScope::All);
    Ok(MergeOutcome {
        text: session.result_text(),
        conflicts: session.counts().conflicts,
    })
}
