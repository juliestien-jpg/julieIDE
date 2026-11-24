//! Tampon de texte : une corde (`ropey`) et son historique d'annulation.
//!
//! Positions en **caractères**. Les lignes sont séparées par `\n`, `\r\n` ou `\r`
//! (comme `julie_core::file`) ; le contenu d'une ligne exclut sa fin de ligne.

use std::collections::VecDeque;
use std::ops::Range;
use std::time::{Duration, Instant};

use julie_core::TextPosition;
use ropey::{Rope, RopeSlice};

use crate::selection::Selections;

/// Pause au-delà de laquelle une frappe continue commence une nouvelle étape
/// d'annulation.
pub const GROUP_PAUSE: Duration = Duration::from_secs(1);

/// Nombre de modifications gardées pour suivre une position ([`Buffer::map_offset_since`]).
const LOG_CAPACITY: usize = 4096;

/// Côté vers lequel une position glisse quand du texte est inséré exactement là.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    /// Reste avant le texte inséré.
    Before,
    /// Passe après le texte inséré.
    After,
}

/// Une modification élémentaire : à `start`, `removed` a été remplacé par
/// `inserted`. Positions en caractères, dans le texte tel qu'il était juste
/// avant cette modification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub removed: String,
    pub inserted: String,
    removed_chars: usize,
    inserted_chars: usize,
}

impl Change {
    pub fn new(start: usize, removed: String, inserted: String) -> Self {
        let removed_chars = removed.chars().count();
        let inserted_chars = inserted.chars().count();
        Self {
            start,
            removed,
            inserted,
            removed_chars,
            inserted_chars,
        }
    }

    pub fn removed_chars(&self) -> usize {
        self.removed_chars
    }

    pub fn inserted_chars(&self) -> usize {
        self.inserted_chars
    }

    fn inverse(&self) -> Change {
        Change {
            start: self.start,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
            removed_chars: self.inserted_chars,
            inserted_chars: self.removed_chars,
        }
    }

    /// Où se retrouve `offset` (position d'avant) une fois la modification faite.
    pub fn map_offset(&self, offset: usize, bias: Bias) -> usize {
        let old_end = self.start + self.removed_chars;
        if offset < self.start || (offset == self.start && bias == Bias::Before) {
            offset
        } else if offset >= old_end && offset > self.start {
            offset - self.removed_chars + self.inserted_chars
        } else {
            // Dans le texte supprimé, ou exactement au point d'insertion.
            match bias {
                Bias::Before => self.start,
                Bias::After => self.start + self.inserted_chars,
            }
        }
    }
}

/// Position en octets, au sens de tree-sitter : ligne, et octet dans la ligne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BytePoint {
    pub row: usize,
    pub column: usize,
}

/// Une modification décrite en octets, pour l'analyse incrémentale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start_point: BytePoint,
    pub old_end_point: BytePoint,
    pub new_end_point: BytePoint,
}

/// Nature d'une modification, pour le groupement des étapes d'annulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Frappe de caractères : les frappes successives forment une seule étape.
    Typing,
    /// Effacement (Retour arrière, Suppr) : idem.
    Deleting,
    /// Tout le reste (collage, saut de ligne, opération sur les lignes…) : une
    /// étape à elle seule.
    Other,
}

#[derive(Debug, Clone)]
struct Group {
    id: u64,
    /// Dans l'ordre où elles ont été appliquées.
    changes: Vec<Change>,
    before: Selections,
    after: Selections,
    kind: EditKind,
    last: Instant,
    sealed: bool,
}

/// Le texte, sa version et son historique.
#[derive(Debug, Clone)]
pub struct Buffer {
    rope: Rope,
    version: u64,
    undo: Vec<Group>,
    redo: Vec<Group>,
    next_group_id: u64,
    saved_state: Option<u64>,
    pending_byte_edits: Vec<ByteEdit>,
    log: VecDeque<(u64, Change)>,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new("")
    }
}

impl Buffer {
    /// Un tampon non modifié contenant `text`.
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            next_group_id: 1,
            saved_state: Some(0),
            pending_byte_edits: Vec::new(),
            log: VecDeque::new(),
        }
    }

    // ----- Lecture -------------------------------------------------------

    /// La corde. `Rope::clone` est en temps constant (nœuds partagés) : c'est
    /// ainsi qu'un instantané part vers un fil de fond.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Augmente à chaque modification (annulation comprise).
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Nombre de lignes ; un texte qui finit par une fin de ligne a une dernière
    /// ligne vide.
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn slice(&self, range: Range<usize>) -> RopeSlice<'_> {
        self.rope.slice(range)
    }

    pub fn char_at(&self, offset: usize) -> Option<char> {
        (offset < self.rope.len_chars()).then(|| self.rope.char(offset))
    }

    /// Ligne `line` avec sa fin de ligne.
    pub fn line_with_ending(&self, line: usize) -> RopeSlice<'_> {
        self.rope.line(line)
    }

    /// Ligne `line` sans sa fin de ligne.
    pub fn line(&self, line: usize) -> RopeSlice<'_> {
        let slice = self.rope.line(line);
        let len = slice.len_chars() - line_ending_len(slice);
        slice.slice(..len)
    }

    /// Longueur en caractères de la ligne, fin de ligne exclue.
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        slice.len_chars() - line_ending_len(slice)
    }

    /// Fin de ligne de `line` (`""` pour la dernière ligne sans saut final).
    pub fn line_ending(&self, line: usize) -> &'static str {
        let slice = self.rope.line(line);
        match line_ending_len(slice) {
            0 => "",
            2 => "\r\n",
            _ if slice.char(slice.len_chars() - 1) == '\r' => "\r",
            _ => "\n",
        }
    }

    /// Premier caractère de la ligne.
    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line.min(self.rope.len_lines()))
    }

    /// Position juste avant la fin de ligne.
    pub fn line_end(&self, line: usize) -> usize {
        self.line_start(line) + self.line_len(line)
    }

    pub fn line_of(&self, offset: usize) -> usize {
        self.rope.char_to_line(offset.min(self.rope.len_chars()))
    }

    /// Position (ligne, colonne en caractères). Une position tombée entre `\r`
    /// et `\n` est ramenée à la fin du contenu de la ligne.
    pub fn offset_to_position(&self, offset: usize) -> TextPosition {
        let offset = offset.min(self.rope.len_chars());
        let line = self.rope.char_to_line(offset);
        let column = (offset - self.rope.line_to_char(line)).min(self.line_len(line));
        TextPosition::new(line as u32, column as u32)
    }

    /// Inverse de [`Self::offset_to_position`] ; ligne et colonne sont bornées.
    pub fn position_to_offset(&self, position: TextPosition) -> usize {
        let line = (position.line as usize).min(self.len_lines() - 1);
        self.line_start(line) + (position.column as usize).min(self.line_len(line))
    }

    pub fn char_to_byte(&self, offset: usize) -> usize {
        self.rope.char_to_byte(offset.min(self.rope.len_chars()))
    }

    pub fn byte_to_char(&self, byte: usize) -> usize {
        self.rope.byte_to_char(byte.min(self.rope.len_bytes()))
    }

    // ----- État enregistré -----------------------------------------------

    /// Identifiant de l'état courant de l'historique : deux moments où il est
    /// égal ont le même texte.
    pub fn state_id(&self) -> u64 {
        self.undo.last().map_or(0, |group| group.id)
    }

    /// Le texte diffère-t-il de celui du dernier chargement ou enregistrement ?
    pub fn is_modified(&self) -> bool {
        self.saved_state != Some(self.state_id())
    }

    /// Note que l'état `state_id` est celui du disque (après un enregistrement
    /// réussi de cet état).
    pub fn mark_saved(&mut self, state_id: u64) {
        self.saved_state = Some(state_id);
    }

    /// Force la marque « modifié » (le fichier a disparu du disque, par exemple).
    pub fn mark_unsaved(&mut self) {
        self.saved_state = None;
    }

    // ----- Modification --------------------------------------------------

    /// Clôt l'étape d'annulation en cours : la prochaine modification en ouvrira
    /// une nouvelle.
    pub fn seal(&mut self) {
        if let Some(group) = self.undo.last_mut() {
            group.sealed = true;
        }
    }

    /// Remplace tout le texte, historique vidé, état « non modifié ».
    pub fn reset(&mut self, text: &str) {
        let old_end_byte = self.rope.len_bytes();
        let old_end_point = self.byte_point(old_end_byte);
        self.rope = Rope::from_str(text);
        self.version += 1;
        self.undo.clear();
        self.redo.clear();
        self.saved_state = Some(0);
        self.log.clear();
        let new_end_byte = self.rope.len_bytes();
        self.pending_byte_edits.push(ByteEdit {
            start_byte: 0,
            old_end_byte,
            new_end_byte,
            start_point: BytePoint::default(),
            old_end_point,
            new_end_point: self.byte_point(new_end_byte),
        });
    }

    /// Applique une transaction : des remplacements `(plage, texte)` triés et
    /// sans chevauchement, exprimés dans le texte **d'avant** la transaction.
    /// `before` et `after` sont les sélections à rétablir à l'annulation et au
    /// rétablissement. Les frappes ([`EditKind::Typing`]) et effacements
    /// successifs, sans déplacement du curseur ni pause de plus de
    /// [`GROUP_PAUSE`], rejoignent l'étape précédente.
    pub fn apply(
        &mut self,
        edits: &[(Range<usize>, String)],
        kind: EditKind,
        before: &Selections,
        after: Selections,
        now: Instant,
    ) {
        debug_assert!(edits.windows(2).all(|w| w[0].0.end <= w[1].0.start));
        let mut changes = Vec::with_capacity(edits.len());
        // De la fin vers le début : les positions des remplacements restants ne
        // bougent pas.
        for (range, text) in edits.iter().rev() {
            let range = range.start.min(self.len_chars())..range.end.min(self.len_chars());
            if range.is_empty() && text.is_empty() {
                continue;
            }
            let removed = self.rope.slice(range.clone()).to_string();
            let change = Change::new(range.start, removed, text.clone());
            self.apply_change(&change);
            changes.push(change);
        }
        if changes.is_empty() {
            return;
        }
        self.redo.clear();

        let merge = match self.undo.last() {
            Some(group) => {
                kind != EditKind::Other
                    && !group.sealed
                    && group.kind == kind
                    && now.saturating_duration_since(group.last) <= GROUP_PAUSE
                    && group.after == *before
            }
            None => false,
        };
        if merge {
            let group = self.undo.last_mut().expect("groupe vérifié");
            group.changes.extend(changes);
            group.after = after;
            group.last = now;
        } else {
            self.seal();
            let id = self.next_group_id;
            self.next_group_id += 1;
            self.undo.push(Group {
                id,
                changes,
                before: before.clone(),
                after,
                kind,
                last: now,
                sealed: kind == EditKind::Other,
            });
        }
    }

    /// Annule la dernière étape ; rend les sélections d'avant cette étape.
    pub fn undo(&mut self) -> Option<Selections> {
        let mut group = self.undo.pop()?;
        group.sealed = true;
        for change in group.changes.iter().rev() {
            self.apply_change(&change.inverse());
        }
        let selections = group.before.clone();
        self.redo.push(group);
        Some(selections)
    }

    /// Rétablit l'étape annulée ; rend les sélections d'après cette étape.
    pub fn redo(&mut self) -> Option<Selections> {
        let group = self.redo.pop()?;
        for change in &group.changes {
            self.apply_change(change);
        }
        let selections = group.after.clone();
        self.undo.push(group);
        Some(selections)
    }

    /// Corrige les sélections à rétablir après la dernière étape (quand elles ne
    /// se calculent qu'une fois le texte modifié).
    pub fn set_last_after(&mut self, after: Selections) {
        if let Some(group) = self.undo.last_mut() {
            group.after = after;
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Les modifications en octets faites depuis le dernier appel (pour
    /// l'analyse incrémentale).
    pub fn take_byte_edits(&mut self) -> Vec<ByteEdit> {
        std::mem::take(&mut self.pending_byte_edits)
    }

    /// Ancre : où se trouve aujourd'hui la position `offset` prise à la version
    /// `version`. `None` si cette version est trop ancienne pour le journal
    /// (plus de 4096 modifications, ou rechargement depuis).
    pub fn map_offset_since(&self, offset: usize, version: u64, bias: Bias) -> Option<usize> {
        if version == self.version {
            return Some(offset.min(self.len_chars()));
        }
        let first = self.log.front().map(|(v, _)| *v)?;
        // L'entrée de version v fait passer de v - 1 à v.
        if version + 1 < first {
            return None;
        }
        let mut offset = offset;
        for (_, change) in self.log.iter().filter(|(v, _)| *v > version) {
            offset = change.map_offset(offset, bias);
        }
        Some(offset.min(self.len_chars()))
    }

    fn apply_change(&mut self, change: &Change) {
        let start_byte = self.rope.char_to_byte(change.start);
        let old_end_byte = self.rope.char_to_byte(change.start + change.removed_chars);
        let start_point = self.byte_point(start_byte);
        let old_end_point = self.byte_point(old_end_byte);

        if change.removed_chars > 0 {
            self.rope
                .remove(change.start..change.start + change.removed_chars);
        }
        if !change.inserted.is_empty() {
            self.rope.insert(change.start, &change.inserted);
        }
        self.version += 1;

        let new_end_byte = start_byte + change.inserted.len();
        self.pending_byte_edits.push(ByteEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_point,
            old_end_point,
            new_end_point: self.byte_point(new_end_byte),
        });
        if self.log.len() == LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back((self.version, change.clone()));
    }

    fn byte_point(&self, byte: usize) -> BytePoint {
        let row = self.rope.byte_to_line(byte);
        BytePoint {
            row,
            column: byte - self.rope.line_to_byte(row),
        }
    }
}

/// Longueur en caractères de la fin de ligne qui termine `slice` (0, 1 ou 2).
pub(crate) fn line_ending_len(slice: RopeSlice<'_>) -> usize {
    let len = slice.len_chars();
    if len == 0 {
        return 0;
    }
    match slice.char(len - 1) {
        '\n' if len >= 2 && slice.char(len - 2) == '\r' => 2,
        '\n' | '\r' => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Selection;

    fn sel(offset: usize) -> Selections {
        Selections::single(Selection::cursor(offset))
    }

    #[test]
    fn lines_exclude_their_ending_whatever_it_is() {
        let buffer = Buffer::new("a\r\nbé\rc\n");
        assert_eq!(buffer.len_lines(), 4);
        assert_eq!(buffer.line(0).to_string(), "a");
        assert_eq!(buffer.line(1).to_string(), "bé");
        assert_eq!(buffer.line_ending(0), "\r\n");
        assert_eq!(buffer.line_ending(1), "\r");
        assert_eq!(buffer.line_ending(2), "\n");
        assert_eq!(buffer.line_ending(3), "");
        // Entre \r et \n : ramené à la fin du contenu.
        assert_eq!(buffer.offset_to_position(2), TextPosition::new(0, 1));
        assert_eq!(buffer.position_to_offset(TextPosition::new(1, 99)), 5);
    }

    #[test]
    fn typing_groups_until_a_pause_or_a_cursor_move() {
        let mut buffer = Buffer::new("");
        let t0 = Instant::now();
        for (i, c) in "abc".chars().enumerate() {
            buffer.apply(
                &[(i..i, c.to_string())],
                EditKind::Typing,
                &sel(i),
                sel(i + 1),
                t0 + Duration::from_millis(100 * i as u64),
            );
        }
        assert!(buffer.is_modified());
        // Pause de plus d'une seconde : nouvelle étape.
        buffer.apply(
            &[(3..3, "d".into())],
            EditKind::Typing,
            &sel(3),
            sel(4),
            t0 + Duration::from_secs(3),
        );
        // Curseur déplacé (before différent de l'after précédent) : nouvelle étape.
        buffer.apply(
            &[(0..0, "x".into())],
            EditKind::Typing,
            &sel(0),
            sel(1),
            t0 + Duration::from_millis(3100),
        );
        assert_eq!(buffer.text(), "xabcd");
        assert_eq!(buffer.undo(), Some(sel(0)));
        assert_eq!(buffer.text(), "abcd");
        buffer.undo();
        assert_eq!(buffer.text(), "abc");
        buffer.undo();
        assert_eq!(buffer.text(), "");
        assert!(!buffer.is_modified());
        assert_eq!(buffer.redo(), Some(sel(3)));
        assert_eq!(buffer.text(), "abc");
    }

    #[test]
    fn multi_edit_transactions_undo_as_one_step() {
        let mut buffer = Buffer::new("un deux trois");
        buffer.apply(
            &[(0..2, "1".into()), (3..7, "2".into()), (8..13, "3".into())],
            EditKind::Other,
            &sel(0),
            sel(0),
            Instant::now(),
        );
        assert_eq!(buffer.text(), "1 2 3");
        buffer.undo();
        assert_eq!(buffer.text(), "un deux trois");
        buffer.redo();
        assert_eq!(buffer.text(), "1 2 3");
    }

    #[test]
    fn saving_closes_the_group_so_later_typing_marks_modified() {
        let mut buffer = Buffer::new("");
        let now = Instant::now();
        buffer.apply(
            &[(0..0, "a".into())],
            EditKind::Typing,
            &sel(0),
            sel(1),
            now,
        );
        buffer.seal();
        buffer.mark_saved(buffer.state_id());
        assert!(!buffer.is_modified());
        buffer.apply(
            &[(1..1, "b".into())],
            EditKind::Typing,
            &sel(1),
            sel(2),
            now,
        );
        assert!(buffer.is_modified());
        buffer.undo();
        assert!(!buffer.is_modified());
        // Une nouvelle branche après annulation ne retombe jamais sur l'état enregistré.
        buffer.undo();
        buffer.apply(&[(0..0, "z".into())], EditKind::Other, &sel(0), sel(1), now);
        assert!(buffer.is_modified());
    }

    #[test]
    fn byte_edits_describe_each_change() {
        let mut buffer = Buffer::new("é\nab");
        buffer.apply(
            &[(3..3, "xy\n".into())],
            EditKind::Other,
            &sel(3),
            sel(6),
            Instant::now(),
        );
        let edits = buffer.take_byte_edits();
        assert_eq!(edits.len(), 1);
        let edit = edits[0];
        assert_eq!(edit.start_byte, 4);
        assert_eq!(edit.old_end_byte, 4);
        assert_eq!(edit.new_end_byte, 7);
        assert_eq!(edit.start_point, BytePoint { row: 1, column: 1 });
        assert_eq!(edit.new_end_point, BytePoint { row: 2, column: 0 });
        assert!(buffer.take_byte_edits().is_empty());
    }

    #[test]
    fn anchors_follow_the_changes() {
        let mut buffer = Buffer::new("abcdef");
        let version = buffer.version();
        let now = Instant::now();
        buffer.apply(
            &[(0..0, "XX".into())],
            EditKind::Other,
            &sel(0),
            sel(2),
            now,
        );
        buffer.apply(&[(4..6, "".into())], EditKind::Other, &sel(4), sel(4), now);
        // « e » était en 4 : +2 puis le texte avant lui a perdu « cd ».
        assert_eq!(buffer.map_offset_since(4, version, Bias::After), Some(4));
        assert_eq!(buffer.text(), "XXabef");
        assert_eq!(buffer.map_offset_since(0, version, Bias::Before), Some(0));
        assert_eq!(buffer.map_offset_since(0, version, Bias::After), Some(2));
        assert_eq!(buffer.map_offset_since(3, version, Bias::After), Some(4));
    }
}
