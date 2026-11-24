//! Session de fusion : l'état que le dialogue de fusion à trois volets pilote.
//!
//! La session possède les trois textes, la liste des blocs et le **résultat**
//! (le volet du centre), qui part de la base. L'interface n'a qu'à relayer les
//! gestes : accepter, ignorer, appliquer les non conflictuels, frappe libre,
//! annuler, rétablir. Tout est synchrone et rapide (pas d'accès disque).

use std::ops::Range;

use julie_core::{CancelToken, TextPosition};

use crate::diff::HunkKind;
use crate::merge::{merge_chunks, Chunk, ChunkKind};
use crate::text::{LineEnding, LineEndingStyle, LineTable, TextLines, BOM};
use crate::DiffError;

/// Côté d'un bloc de fusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Notre version (volet de gauche).
    Left,
    /// Leur version (volet de droite).
    Right,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Nature d'un bloc à traiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// Changement de gauche seulement.
    Left,
    /// Changement de droite seulement.
    Right,
    /// Même changement des deux côtés (une seule paire accepter/ignorer).
    Identical,
    /// Conflit.
    Conflict,
}

impl BlockKind {
    pub fn is_conflict(self) -> bool {
        self == BlockKind::Conflict
    }
}

/// État d'un côté d'un bloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SideState {
    /// Ce côté n'a pas de changement dans ce bloc : ni flèche ni croix.
    NotApplicable,
    /// En attente.
    Pending,
    /// Son texte a été mis dans le résultat.
    Accepted,
    /// Écarté sans toucher au résultat.
    Ignored,
}

/// Portée de « appliquer les changements non conflictuels ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplyScope {
    /// Changements de gauche, de droite et identiques.
    All,
    /// Changements de gauche et identiques.
    Left,
    /// Changements de droite et identiques.
    Right,
}

/// Vue d'un bloc à un instant donné.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Indice du bloc (stable pendant toute la session).
    pub id: usize,
    pub kind: BlockKind,
    /// Lignes de la base.
    pub base: Range<usize>,
    /// Lignes de notre version (volet de gauche).
    pub ours: Range<usize>,
    /// Lignes de leur version (volet de droite).
    pub theirs: Range<usize>,
    /// Lignes du résultat occupées par le bloc **en ce moment**.
    pub result: Range<usize>,
    pub left: SideState,
    pub right: SideState,
    /// Le bloc ne compte plus dans les « restants ».
    pub resolved: bool,
}

impl Block {
    pub fn state(&self, side: Side) -> SideState {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    /// Forme du changement de ce côté par rapport à la base, pour la couleur du
    /// bloc dans son volet : ajout (rien dans la base), suppression (plus rien de
    /// ce côté) ou modification. N'a de sens que pour un côté qui a un changement
    /// dans ce bloc (état autre que `NotApplicable`).
    pub fn shape(&self, side: Side) -> HunkKind {
        let lines = match side {
            Side::Left => &self.ours,
            Side::Right => &self.theirs,
        };
        if self.base.is_empty() {
            HunkKind::Added
        } else if lines.is_empty() {
            HunkKind::Removed
        } else {
            HunkKind::Modified
        }
    }

    /// La flèche de ce côté est active.
    pub fn can_accept(&self, side: Side) -> bool {
        matches!(self.state(side), SideState::Pending | SideState::Ignored)
    }

    /// La flèche de ce côté signifie « ajouter à la suite » (l'autre côté d'un
    /// conflit est déjà accepté).
    pub fn accept_appends(&self, side: Side) -> bool {
        self.can_accept(side) && self.state(side.other()) == SideState::Accepted
    }

    /// La croix de ce côté est active.
    pub fn can_ignore(&self, side: Side) -> bool {
        self.state(side) == SideState::Pending
    }
}

/// Compteur « n changements et m conflits restants ».
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MergeCounts {
    /// Blocs non conflictuels en attente.
    pub changes: usize,
    /// Conflits en attente.
    pub conflicts: usize,
}

impl MergeCounts {
    pub fn is_done(&self) -> bool {
        self.changes == 0 && self.conflicts == 0
    }
}

/// Tronçon de la correspondance entre les trois volets. Les tronçons se suivent
/// sans trou. Pour un tronçon inchangé (`block` à `None`), `left` et `right` ont
/// la même longueur ; `result` aussi tant qu'on n'y a pas tapé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignedSegment {
    pub left: Range<usize>,
    pub result: Range<usize>,
    pub right: Range<usize>,
    pub block: Option<usize>,
}

/// Styles de fins de ligne des trois versions, pour le bandeau d'information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineEndingReport {
    pub base: LineEndingStyle,
    pub ours: LineEndingStyle,
    pub theirs: LineEndingStyle,
    /// Fin de ligne donnée aux lignes du résultat qui ne viennent pas telles
    /// quelles de notre version (style dominant de notre version).
    pub result: LineEnding,
    /// Les styles diffèrent entre les versions : afficher le bandeau.
    pub differ: bool,
}

/// Refus d'une action sur la session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("bloc de fusion inconnu : {0}")]
    UnknownBlock(usize),
    #[error("ce côté n'a pas de changement dans ce bloc")]
    SideNotApplicable,
    #[error("ce côté est déjà accepté")]
    AlreadyAccepted,
    #[error("ce côté est déjà ignoré")]
    AlreadyIgnored,
    #[error("ce bloc n'est pas un conflit")]
    NotAConflict,
    #[error("plage hors du résultat")]
    OutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResultLine {
    Base(u32),
    Ours(u32),
    Theirs(u32),
    Typed(Box<str>),
}

#[derive(Debug, Clone)]
struct Source {
    text: String,
    table: LineTable,
}

impl Source {
    fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            table: LineTable::new(text),
        }
    }

    fn lines(&self) -> TextLines<'_> {
        TextLines::from_parts(&self.text, self.table.clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockState {
    left: SideState,
    right: SideState,
}

#[derive(Debug, Clone)]
struct BlockData {
    kind: BlockKind,
    base: Range<usize>,
    ours: Range<usize>,
    theirs: Range<usize>,
    state: BlockState,
}

impl BlockData {
    fn resolved(&self) -> bool {
        let BlockState { left, right } = self.state;
        match self.kind {
            BlockKind::Left => left != SideState::Pending,
            BlockKind::Right => right != SideState::Pending,
            BlockKind::Identical => left != SideState::Pending,
            BlockKind::Conflict => {
                left == SideState::Accepted
                    || right == SideState::Accepted
                    || (left == SideState::Ignored && right == SideState::Ignored)
            }
        }
    }
}

/// Remplacement de lignes du résultat, avec de quoi le défaire.
#[derive(Debug, Clone)]
struct Splice {
    before_start: usize,
    removed: Vec<ResultLine>,
    after_start: usize,
    inserted: Vec<ResultLine>,
}

/// Un geste élémentaire : des remplacements simultanés et leurs effets sur les
/// blocs.
#[derive(Debug, Clone, Default)]
struct Part {
    splices: Vec<Splice>,
    /// (indice de segment, longueur avant, longueur après).
    segments: Vec<(usize, usize, usize)>,
    /// (bloc, état avant, état après).
    states: Vec<(usize, BlockState, BlockState)>,
}

/// Une étape annulable : un ou plusieurs gestes, rejoués dans l'ordre.
#[derive(Debug, Clone, Default)]
struct Step {
    parts: Vec<Part>,
}

/// Session de fusion à trois volets.
///
/// ```
/// use julie_diff::{ApplyScope, MergeSession, Side};
///
/// let cancel = julie_core::CancelToken::new();
/// let mut session = MergeSession::new("a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\nd\n", &cancel).unwrap();
/// assert_eq!(session.counts().changes, 2);
/// session.apply_non_conflicting(ApplyScope::All);
/// assert!(session.is_all_resolved());
/// assert_eq!(session.result_text(), "a\nB\nc\nd\n");
/// session.undo();
/// assert_eq!(session.result_text(), "a\nb\nc\n");
/// ```
#[derive(Debug, Clone)]
pub struct MergeSession {
    base: Source,
    ours: Source,
    theirs: Source,
    chunks: Vec<Chunk>,
    blocks: Vec<BlockData>,
    /// Longueurs, en lignes du résultat : `[intervalle 0, bloc 0, intervalle 1,
    /// bloc 1, …, intervalle n]`.
    segments: Vec<usize>,
    result: Vec<ResultLine>,
    undo: Vec<Step>,
    redo: Vec<Step>,
    dominant: LineEnding,
}

impl MergeSession {
    /// Ouvre une session. `ours` est notre version (gauche), `theirs` la leur
    /// (droite) ; sans ancêtre commun, passer une base vide. Rien n'est appliqué :
    /// le résultat est la base. Seule erreur possible : l'annulation.
    pub fn new(
        base: &str,
        ours: &str,
        theirs: &str,
        cancel: &CancelToken,
    ) -> Result<Self, DiffError> {
        let base = Source::new(base);
        let ours = Source::new(ours);
        let theirs = Source::new(theirs);
        let chunks = merge_chunks(&base.lines(), &ours.lines(), &theirs.lines(), cancel)?;

        let mut blocks = Vec::new();
        let mut segments = vec![0usize];
        let mut result = Vec::with_capacity(base.table.len());
        for chunk in &chunks {
            let kind = match chunk.kind {
                ChunkKind::Unchanged => {
                    // Identiques à notre version : on garde ses fins de ligne.
                    result.extend(chunk.ours.clone().map(|i| ResultLine::Ours(i as u32)));
                    *segments.last_mut().expect("segments non vide") += chunk.ours.len();
                    continue;
                }
                ChunkKind::Left => BlockKind::Left,
                ChunkKind::Right => BlockKind::Right,
                ChunkKind::Identical => BlockKind::Identical,
                ChunkKind::Conflict => BlockKind::Conflict,
            };
            let state = match kind {
                BlockKind::Left => BlockState {
                    left: SideState::Pending,
                    right: SideState::NotApplicable,
                },
                BlockKind::Right => BlockState {
                    left: SideState::NotApplicable,
                    right: SideState::Pending,
                },
                BlockKind::Identical | BlockKind::Conflict => BlockState {
                    left: SideState::Pending,
                    right: SideState::Pending,
                },
            };
            result.extend(chunk.base.clone().map(|i| ResultLine::Base(i as u32)));
            segments.push(chunk.base.len());
            segments.push(0);
            blocks.push(BlockData {
                kind,
                base: chunk.base.clone(),
                ours: chunk.ours.clone(),
                theirs: chunk.theirs.clone(),
                state,
            });
        }

        let dominant = ours
            .table
            .dominant_ending()
            .or_else(|| theirs.table.dominant_ending())
            .or_else(|| base.table.dominant_ending())
            .unwrap_or(LineEnding::Lf);

        Ok(Self {
            base,
            ours,
            theirs,
            chunks,
            blocks,
            segments,
            result,
            undo: Vec::new(),
            redo: Vec::new(),
            dominant,
        })
    }

    // ----- Lecture ---------------------------------------------------------

    /// Les lignes de la base.
    pub fn base(&self) -> TextLines<'_> {
        self.base.lines()
    }

    /// Les lignes de notre version (volet de gauche).
    pub fn ours(&self) -> TextLines<'_> {
        self.ours.lines()
    }

    /// Les lignes de leur version (volet de droite).
    pub fn theirs(&self) -> TextLines<'_> {
        self.theirs.lines()
    }

    /// Le découpage d'origine en tronçons classés (ne change jamais).
    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    /// Nombre de blocs (changements et conflits, traités ou non).
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Tous les blocs, dans l'ordre du fichier, avec leur place actuelle dans le
    /// résultat.
    pub fn blocks(&self) -> Vec<Block> {
        let mut start = 0;
        self.blocks
            .iter()
            .enumerate()
            .map(|(id, data)| {
                start += self.segments[2 * id];
                let len = self.segments[2 * id + 1];
                let block = Self::view(id, data, start..start + len);
                start += len;
                block
            })
            .collect()
    }

    /// Un bloc.
    pub fn block(&self, id: usize) -> Option<Block> {
        let data = self.blocks.get(id)?;
        Some(Self::view(id, data, self.block_range(id)))
    }

    fn view(id: usize, data: &BlockData, result: Range<usize>) -> Block {
        Block {
            id,
            kind: data.kind,
            base: data.base.clone(),
            ours: data.ours.clone(),
            theirs: data.theirs.clone(),
            result,
            left: data.state.left,
            right: data.state.right,
            resolved: data.resolved(),
        }
    }

    fn block_range(&self, id: usize) -> Range<usize> {
        let start: usize = self.segments[..2 * id + 1].iter().sum();
        start..start + self.segments[2 * id + 1]
    }

    /// Blocs restant à traiter.
    pub fn counts(&self) -> MergeCounts {
        let mut counts = MergeCounts::default();
        for block in self.blocks.iter().filter(|b| !b.resolved()) {
            if block.kind.is_conflict() {
                counts.conflicts += 1;
            } else {
                counts.changes += 1;
            }
        }
        counts
    }

    /// « Tout est traité » : plus aucun bloc en attente.
    pub fn is_all_resolved(&self) -> bool {
        self.counts().is_done()
    }

    /// Quelque chose a été fait depuis l'ouverture (et n'a pas été annulé) :
    /// sert à décider s'il faut confirmer la fermeture.
    pub fn is_touched(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Nombre de lignes du résultat.
    pub fn result_line_count(&self) -> usize {
        self.result.len()
    }

    /// Contenu d'une ligne du résultat, sans fin de ligne.
    pub fn result_line(&self, line: usize) -> &str {
        match &self.result[line] {
            ResultLine::Base(i) => self.base.table.content(&self.base.text, *i as usize),
            ResultLine::Ours(i) => self.ours.table.content(&self.ours.text, *i as usize),
            ResultLine::Theirs(i) => self.theirs.table.content(&self.theirs.text, *i as usize),
            ResultLine::Typed(text) => text,
        }
    }

    /// Les lignes du résultat, sans fins de ligne (pour l'affichage du centre).
    pub fn result_lines(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.result.len()).map(|line| self.result_line(line))
    }

    fn line_ending(&self, line: usize) -> LineEnding {
        let last = line + 1 == self.result.len();
        let own = match &self.result[line] {
            // Restée identique à notre version : sa propre fin de ligne.
            ResultLine::Ours(i) => self.ours.table.ending(*i as usize),
            // Venue d'ailleurs : style dominant, si la source avait une fin de ligne.
            ResultLine::Base(i) => self.styled(self.base.table.ending(*i as usize)),
            ResultLine::Theirs(i) => self.styled(self.theirs.table.ending(*i as usize)),
            // Tapée : en dernière position, suit le saut final de notre version.
            ResultLine::Typed(_) => {
                if !last || self.ours.table.has_final_newline() {
                    self.dominant
                } else {
                    LineEnding::None
                }
            }
        };
        if !last && own == LineEnding::None {
            self.dominant
        } else {
            own
        }
    }

    fn styled(&self, ending: LineEnding) -> LineEnding {
        match ending {
            LineEnding::None => LineEnding::None,
            _ => self.dominant,
        }
    }

    /// Le texte du résultat tel qu'il doit être écrit : fins de ligne et saut de
    /// ligne final selon la règle de la spécification, marque BOM en tête si
    /// notre version en avait une (voir [`MergeSession::result_has_bom`]).
    pub fn result_text(&self) -> String {
        let mut out = String::new();
        if self.result_has_bom() {
            out.push(BOM);
        }
        for line in 0..self.result.len() {
            out.push_str(self.result_line(line));
            out.push_str(self.line_ending(line).as_str());
        }
        out
    }

    /// Le texte de notre version commençait par une marque BOM ; le résultat en
    /// porte donc une. (Si l'appelant a décodé les fichiers avec
    /// `julie_core::file`, la BOM est déjà dans le `TextFormat` et ceci rend faux.)
    pub fn result_has_bom(&self) -> bool {
        self.ours.table.has_bom
    }

    /// Styles de fins de ligne des trois versions.
    pub fn line_endings(&self) -> LineEndingReport {
        let base = self.base.table.style();
        let ours = self.ours.table.style();
        let theirs = self.theirs.table.style();
        let mut seen: Vec<LineEndingStyle> = Vec::new();
        for style in [base, ours, theirs] {
            if style != LineEndingStyle::None && !seen.contains(&style) {
                seen.push(style);
            }
        }
        LineEndingReport {
            base,
            ours,
            theirs,
            result: self.dominant,
            differ: seen.len() > 1,
        }
    }

    /// Correspondance complète entre les trois volets : connecteurs et défilement
    /// synchronisé.
    pub fn alignment(&self) -> Vec<AlignedSegment> {
        let mut out = Vec::with_capacity(self.segments.len());
        let (mut left, mut right, mut result) = (0, 0, 0);
        for (id, block) in self.blocks.iter().enumerate() {
            let gap = self.segments[2 * id];
            if block.ours.start > left || gap > 0 {
                out.push(AlignedSegment {
                    left: left..block.ours.start,
                    result: result..result + gap,
                    right: right..block.theirs.start,
                    block: None,
                });
            }
            result += gap;
            let len = self.segments[2 * id + 1];
            out.push(AlignedSegment {
                left: block.ours.clone(),
                result: result..result + len,
                right: block.theirs.clone(),
                block: Some(id),
            });
            result += len;
            left = block.ours.end;
            right = block.theirs.end;
        }
        let gap = *self.segments.last().expect("segments non vide");
        if left < self.ours.table.len() || gap > 0 {
            out.push(AlignedSegment {
                left: left..self.ours.table.len(),
                result: result..result + gap,
                right: right..self.theirs.table.len(),
                block: None,
            });
        }
        out
    }

    /// Le bloc qui contient la ligne `line` du résultat. Un bloc vide (zéro ligne
    /// au centre) est rendu pour la ligne où il s'insère.
    pub fn block_at_result_line(&self, line: usize) -> Option<usize> {
        let mut start = 0;
        for id in 0..self.blocks.len() {
            start += self.segments[2 * id];
            let len = self.segments[2 * id + 1];
            if (start..start + len).contains(&line) || (len == 0 && start == line) {
                return Some(id);
            }
            if start > line {
                return None;
            }
            start += len;
        }
        None
    }

    /// Premier bloc **en attente** après `current` (ou le premier de tous si
    /// `current` est `None`). Pas de retour au début : `None` à la fin.
    pub fn next_pending(&self, current: Option<usize>) -> Option<usize> {
        let from = current.map_or(0, |id| id + 1);
        (from..self.blocks.len()).find(|&id| !self.blocks[id].resolved())
    }

    /// Dernier bloc **en attente** avant `current` (ou le dernier de tous si
    /// `current` est `None`).
    pub fn previous_pending(&self, current: Option<usize>) -> Option<usize> {
        let to = current.unwrap_or(self.blocks.len()).min(self.blocks.len());
        (0..to).rev().find(|&id| !self.blocks[id].resolved())
    }

    // ----- Actions ---------------------------------------------------------

    fn side_lines(&self, id: usize, side: Side) -> Vec<ResultLine> {
        let block = &self.blocks[id];
        match (block.kind, side) {
            // Un changement identique s'applique avec notre texte (mêmes lignes,
            // nos fins de ligne).
            (BlockKind::Identical, _) | (_, Side::Left) => block
                .ours
                .clone()
                .map(|i| ResultLine::Ours(i as u32))
                .collect(),
            (_, Side::Right) => block
                .theirs
                .clone()
                .map(|i| ResultLine::Theirs(i as u32))
                .collect(),
        }
    }

    fn check_side(&self, id: usize, side: Side) -> Result<SideState, SessionError> {
        let block = self.blocks.get(id).ok_or(SessionError::UnknownBlock(id))?;
        let state = match side {
            Side::Left => block.state.left,
            Side::Right => block.state.right,
        };
        match state {
            SideState::NotApplicable => Err(SessionError::SideNotApplicable),
            SideState::Accepted => Err(SessionError::AlreadyAccepted),
            other => Ok(other),
        }
    }

    fn with_side(&self, id: usize, side: Side, new: SideState) -> BlockState {
        let block = &self.blocks[id];
        let mut state = block.state;
        if block.kind == BlockKind::Identical {
            state.left = new;
            state.right = new;
        } else {
            match side {
                Side::Left => state.left = new,
                Side::Right => state.right = new,
            }
        }
        state
    }

    /// Prépare l'acceptation d'un côté : (plage du résultat à remplacer, lignes).
    fn plan_accept(&self, id: usize, side: Side) -> (Range<usize>, Vec<ResultLine>) {
        let block = &self.blocks[id];
        let range = self.block_range(id);
        let other = match side {
            Side::Left => block.state.right,
            Side::Right => block.state.left,
        };
        let lines = self.side_lines(id, side);
        if block.kind == BlockKind::Conflict && other == SideState::Accepted {
            // « Ajouter à la suite » : après le texte déjà accepté.
            (range.end..range.end, lines)
        } else {
            (range, lines)
        }
    }

    /// Accepte un côté d'un bloc : son texte remplace celui du bloc dans le
    /// résultat. Sur un conflit dont l'autre côté est déjà accepté, le texte est
    /// **ajouté à la suite** : accepter les deux côtés donne les deux textes dans
    /// l'ordre des clics.
    pub fn accept(&mut self, id: usize, side: Side) -> Result<(), SessionError> {
        self.check_side(id, side)?;
        let (range, lines) = self.plan_accept(id, side);
        let state = self.with_side(id, side, SideState::Accepted);
        let mut step = Part::default();
        self.set_state(id, state, &mut step);
        let new_len = self.segments[2 * id + 1] - range.len() + lines.len();
        self.set_segment(2 * id + 1, new_len, &mut step);
        step.splices = self.apply_splices(vec![(range, lines)]);
        self.commit(step);
        Ok(())
    }

    /// Accepte les deux côtés d'un conflit en une seule étape annulable : le
    /// texte de `first`, puis celui de l'autre côté.
    pub fn accept_both(&mut self, id: usize, first: Side) -> Result<(), SessionError> {
        let block = self.blocks.get(id).ok_or(SessionError::UnknownBlock(id))?;
        if block.kind != BlockKind::Conflict {
            return Err(SessionError::NotAConflict);
        }
        self.check_side(id, first)?;
        self.check_side(id, first.other())?;
        self.accept(id, first)?;
        self.accept(id, first.other())?;
        self.merge_last_two_steps();
        Ok(())
    }

    /// Ignore un côté d'un bloc : il est marqué traité, le résultat ne bouge pas.
    /// Un conflit n'est résolu que si l'autre côté est accepté ou ignoré aussi.
    pub fn ignore(&mut self, id: usize, side: Side) -> Result<(), SessionError> {
        if self.check_side(id, side)? == SideState::Ignored {
            return Err(SessionError::AlreadyIgnored);
        }
        let state = self.with_side(id, side, SideState::Ignored);
        let mut step = Part::default();
        self.set_state(id, state, &mut step);
        self.commit(step);
        Ok(())
    }

    /// Applique tous les changements non conflictuels en attente de la portée
    /// donnée, en une seule étape annulable. Les conflits et les blocs déjà
    /// traités (acceptés ou ignorés) ne sont pas touchés. Rend le nombre de blocs
    /// appliqués.
    pub fn apply_non_conflicting(&mut self, scope: ApplyScope) -> usize {
        let mut step = Part::default();
        let mut splices = Vec::new();
        let mut start = 0;
        for id in 0..self.blocks.len() {
            start += self.segments[2 * id];
            let len = self.segments[2 * id + 1];
            let range = start..start + len;
            start += len;
            let block = &self.blocks[id];
            let side = match (block.kind, scope) {
                (BlockKind::Conflict, _) => continue,
                (BlockKind::Identical, _) => Side::Left,
                (BlockKind::Left, ApplyScope::All | ApplyScope::Left) => Side::Left,
                (BlockKind::Right, ApplyScope::All | ApplyScope::Right) => Side::Right,
                _ => continue,
            };
            if block.resolved() {
                continue;
            }
            let lines = self.side_lines(id, side);
            let state = self.with_side(id, side, SideState::Accepted);
            self.set_state(id, state, &mut step);
            self.set_segment(2 * id + 1, lines.len(), &mut step);
            splices.push((range, lines));
        }
        let applied = splices.len();
        if applied > 0 {
            step.splices = self.apply_splices(splices);
            self.commit(step);
        }
        applied
    }

    /// Frappe libre, par lignes entières : remplace les lignes `range` du
    /// résultat par `lines` (contenus sans fin de ligne). Une plage vide insère ;
    /// `lines` vide supprime. Les lignes identiques en tête et en queue du
    /// remplacement sont laissées telles quelles.
    ///
    /// Taper dans un bloc ne le résout pas ; le bloc suit le texte (il grandit ou
    /// rétrécit) et ses flèches restent actives.
    pub fn replace_lines(
        &mut self,
        range: Range<usize>,
        lines: &[&str],
    ) -> Result<(), SessionError> {
        if range.start > range.end || range.end > self.result.len() {
            return Err(SessionError::OutOfRange);
        }
        // Rogne ce qui ne change pas : ces lignes gardent leur origine.
        let mut start = range.start;
        let mut end = range.end;
        let mut lines = lines;
        while start < end && !lines.is_empty() && self.result_line(start) == lines[0] {
            start += 1;
            lines = &lines[1..];
        }
        while start < end
            && !lines.is_empty()
            && self.result_line(end - 1) == lines[lines.len() - 1]
        {
            end -= 1;
            lines = &lines[..lines.len() - 1];
        }
        if start == end && lines.is_empty() {
            return Ok(());
        }

        let mut step = Part::default();
        let after_previous = start > range.start;
        let before_next = end < range.end;
        self.move_boundaries(
            start,
            end,
            lines.len(),
            after_previous,
            before_next,
            &mut step,
        );
        let inserted = lines
            .iter()
            .map(|line| ResultLine::Typed((*line).into()))
            .collect();
        step.splices = self.apply_splices(vec![(start..end, inserted)]);
        self.commit(step);
        Ok(())
    }

    /// Frappe libre, au caractère : remplace le texte entre `start` et `end`
    /// (lignes du résultat, colonnes en caractères) par `text`. `text` peut
    /// contenir des sauts de ligne (`\n` ou `\r\n`). La position
    /// `(nombre de lignes, 0)` désigne la fin du texte.
    ///
    /// Le saut de ligne final ne s'édite pas : il suit la règle de la
    /// spécification (voir [`MergeSession::result_text`]).
    pub fn edit(
        &mut self,
        start: TextPosition,
        end: TextPosition,
        text: &str,
    ) -> Result<(), SessionError> {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let count = self.result.len();
        let (start_line, end_line) = (start.line as usize, end.line as usize);
        if end_line > count {
            return Err(SessionError::OutOfRange);
        }
        let line_text = |line: usize| {
            if line < count {
                self.result_line(line)
            } else {
                ""
            }
        };
        let byte_at = |line: usize, column: u32| -> Result<usize, SessionError> {
            let content = line_text(line);
            let mut indices = content.char_indices().map(|(i, _)| i);
            match indices.nth(column as usize) {
                Some(i) => Ok(i),
                None if content.chars().count() == column as usize => Ok(content.len()),
                None => Err(SessionError::OutOfRange),
            }
        };
        let prefix = &line_text(start_line)[..byte_at(start_line, start.column)?];
        let suffix = &line_text(end_line)[byte_at(end_line, end.column)?..];
        // Seuls les sauts de ligne du texte tapé comptent ; un `\r` qui précède
        // l'un d'eux part avec lui. Un `\r` isolé ailleurs (dans le texte tapé ou
        // déjà dans la ligne) reste du contenu.
        let mut pieces: Vec<String> = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find('\n') {
            let piece = &rest[..at];
            pieces.push(piece.strip_suffix('\r').unwrap_or(piece).to_owned());
            rest = &rest[at + 1..];
        }
        pieces.push(rest.to_owned());
        pieces[0].insert_str(0, prefix);
        pieces
            .last_mut()
            .expect("au moins une pièce")
            .push_str(suffix);
        let mut new_lines: Vec<&str> = pieces.iter().map(String::as_str).collect();
        let replaced_end = if end_line == count {
            // La dernière pièce est ce qui suit le dernier saut de ligne : vide,
            // elle n'est pas une ligne.
            if new_lines.last().is_some_and(|line| line.is_empty()) {
                new_lines.pop();
            }
            count
        } else {
            end_line + 1
        };
        self.replace_lines(start_line..replaced_end, &new_lines)
    }

    // ----- Annuler, rétablir -----------------------------------------------

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Annule la dernière étape (frappe, accepter, ignorer, appliquer), compteur
    /// compris. Rend `false` s'il n'y a rien à annuler.
    pub fn undo(&mut self) -> bool {
        let Some(step) = self.undo.pop() else {
            return false;
        };
        for part in step.parts.iter().rev() {
            let inverse = part
                .splices
                .iter()
                .map(|s| {
                    (
                        s.after_start..s.after_start + s.inserted.len(),
                        s.removed.clone(),
                    )
                })
                .collect();
            self.apply_splices(inverse);
            for &(index, before, _) in part.segments.iter().rev() {
                self.segments[index] = before;
            }
            for &(id, before, _) in part.states.iter().rev() {
                self.blocks[id].state = before;
            }
        }
        self.redo.push(step);
        true
    }

    /// Rétablit la dernière étape annulée. Rend `false` s'il n'y a rien à rétablir.
    pub fn redo(&mut self) -> bool {
        let Some(step) = self.redo.pop() else {
            return false;
        };
        for part in &step.parts {
            let forward = part
                .splices
                .iter()
                .map(|s| {
                    (
                        s.before_start..s.before_start + s.removed.len(),
                        s.inserted.clone(),
                    )
                })
                .collect();
            self.apply_splices(forward);
            for &(index, _, after) in &part.segments {
                self.segments[index] = after;
            }
            for &(id, _, after) in &part.states {
                self.blocks[id].state = after;
            }
        }
        self.undo.push(step);
        true
    }

    /// Soude la dernière étape annulable à celle d'avant, pour qu'un seul
    /// « annuler » défasse les deux (regrouper une rafale de frappe, par
    /// exemple). Rend `false` s'il y a moins de deux étapes.
    pub fn merge_last_two_steps(&mut self) -> bool {
        if self.undo.len() < 2 {
            return false;
        }
        let last = self.undo.pop().expect("deux étapes");
        // Les remplacements de la seconde étape sont exprimés dans le texte
        // d'après la première : on garde les gestes en séquence.
        self.undo
            .last_mut()
            .expect("deux étapes")
            .parts
            .extend(last.parts);
        true
    }

    // ----- Mécanique interne -----------------------------------------------

    fn commit(&mut self, part: Part) {
        self.redo.clear();
        self.undo.push(Step { parts: vec![part] });
    }

    fn set_state(&mut self, id: usize, state: BlockState, step: &mut Part) {
        let before = self.blocks[id].state;
        if before != state {
            step.states.push((id, before, state));
            self.blocks[id].state = state;
        }
    }

    fn set_segment(&mut self, index: usize, len: usize, step: &mut Part) {
        let before = self.segments[index];
        if before != len {
            step.segments.push((index, before, len));
            self.segments[index] = len;
        }
    }

    /// Met à jour les longueurs des segments pour le remplacement des lignes
    /// `[start, end)` par `count` lignes tapées.
    ///
    /// Une frontière avant le remplacement ne bouge pas ; après, elle suit ;
    /// prise dedans, elle passe après le texte tapé. Une insertion pure exactement
    /// sur une frontière tombe **entre** les blocs, sauf si la frappe venait de
    /// l'intérieur d'un bloc : `after_previous` (la frappe a commencé sur la ligne
    /// d'avant, restée telle quelle) la rattache au bloc qui finit là ;
    /// `before_next` (elle se termine sur la ligne d'après) au bloc qui y commence.
    fn move_boundaries(
        &mut self,
        start: usize,
        end: usize,
        count: usize,
        after_previous: bool,
        before_next: bool,
        step: &mut Part,
    ) {
        let insertion = start == end;
        let map = |position: usize, is_block_start: bool| -> usize {
            if insertion {
                let follows = if position != start {
                    position > start
                } else if is_block_start {
                    after_previous || !before_next
                } else {
                    after_previous
                };
                if follows {
                    position + count
                } else {
                    position
                }
            } else if position <= start {
                position
            } else if position >= end {
                position + count - (end - start)
            } else {
                start + count
            }
        };
        let mut new_segments = Vec::with_capacity(self.segments.len());
        let mut old_position = 0;
        let mut new_position = 0;
        for id in 0..self.blocks.len() {
            let block_start = old_position + self.segments[2 * id];
            let block_end = block_start + self.segments[2 * id + 1];
            let new_start = map(block_start, true);
            // Un bloc vide n'a pas de dernière ligne à laquelle rattacher la
            // frappe : sa fin suit son début.
            let new_end = if block_start == block_end {
                new_start
            } else {
                map(block_end, false)
            };
            debug_assert!(new_start >= new_position && new_end >= new_start);
            new_segments.push(new_start - new_position);
            new_segments.push(new_end - new_start);
            old_position = block_end;
            new_position = new_end;
        }
        let total = self.result.len() + count - (end - start);
        new_segments.push(total - new_position);
        for (index, len) in new_segments.into_iter().enumerate() {
            self.set_segment(index, len, step);
        }
    }

    /// Applique des remplacements triés et disjoints, exprimés dans le résultat
    /// d'avant. Rend de quoi les rejouer dans les deux sens.
    fn apply_splices(&mut self, splices: Vec<(Range<usize>, Vec<ResultLine>)>) -> Vec<Splice> {
        let mut records = Vec::with_capacity(splices.len());
        if let [(range, _)] = splices.as_slice() {
            // Cas courant : un seul remplacement, fait en place.
            let range = range.clone();
            let (_, inserted) = splices.into_iter().next().expect("un remplacement");
            let removed: Vec<ResultLine> = self
                .result
                .splice(range.clone(), inserted.iter().cloned())
                .collect();
            records.push(Splice {
                before_start: range.start,
                removed,
                after_start: range.start,
                inserted,
            });
            return records;
        }
        let old = std::mem::take(&mut self.result);
        let mut new = Vec::with_capacity(old.len());
        let mut old_iter = old.into_iter();
        let mut position = 0;
        for (range, inserted) in splices {
            debug_assert!(range.start >= position, "remplacements non triés");
            new.extend(old_iter.by_ref().take(range.start - position));
            let removed: Vec<ResultLine> = old_iter.by_ref().take(range.len()).collect();
            let after_start = new.len();
            new.extend(inserted.iter().cloned());
            records.push(Splice {
                before_start: range.start,
                removed,
                after_start,
                inserted,
            });
            position = range.end;
        }
        new.extend(old_iter);
        self.result = new;
        records
    }
}
