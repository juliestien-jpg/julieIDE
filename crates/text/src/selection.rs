//! Sélections et curseurs. Le modèle est multiple dès maintenant ; la v0 n'en
//! montre qu'un.

use std::ops::Range;

/// Une sélection : `anchor` est le bout fixe, `head` le curseur. Les deux sont des
/// positions en **caractères** dans le texte (pas en octets). Vide quand
/// `anchor == head`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Colonne visuelle visée par les déplacements verticaux (Haut/Bas), gardée
    /// d'une ligne à l'autre même quand une ligne plus courte la rabote.
    pub goal_column: Option<u32>,
}

impl Selection {
    pub fn cursor(offset: usize) -> Self {
        Self {
            anchor: offset,
            head: offset,
            goal_column: None,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            goal_column: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }

    /// Le curseur va en `offset` ; la sélection s'étend si `extend`, sinon elle
    /// se réduit au curseur.
    pub fn moved_to(self, offset: usize, extend: bool) -> Self {
        Self {
            anchor: if extend { self.anchor } else { offset },
            head: offset,
            goal_column: None,
        }
    }
}

/// L'ensemble des sélections d'un document : jamais vide, triées, sans
/// chevauchement. `primary` est celle que suivent le défilement et la barre d'état.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selections {
    ranges: Vec<Selection>,
    primary: usize,
}

impl Default for Selections {
    fn default() -> Self {
        Self::single(Selection::cursor(0))
    }
}

impl Selections {
    pub fn single(selection: Selection) -> Self {
        Self {
            ranges: vec![selection],
            primary: 0,
        }
    }

    /// Construit l'ensemble en triant et en fusionnant ce qui se chevauche.
    /// `primary` désigne une entrée de `ranges` avant tri.
    pub fn from_vec(ranges: Vec<Selection>, primary: usize) -> Self {
        if ranges.is_empty() {
            return Self::default();
        }
        let primary_selection = ranges[primary.min(ranges.len() - 1)];
        let mut selections = Self { ranges, primary: 0 };
        selections.normalize(primary_selection);
        selections
    }

    pub fn primary(&self) -> Selection {
        self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.ranges.iter()
    }

    pub fn as_slice(&self) -> &[Selection] {
        &self.ranges
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Ajoute un curseur (Alt+clic, plus tard).
    pub fn add(&mut self, selection: Selection) {
        self.ranges.push(selection);
        self.normalize(selection);
    }

    /// Ne garde que la sélection principale.
    pub fn collapse_to_primary(&mut self) {
        let primary = self.primary();
        self.ranges = vec![primary];
        self.primary = 0;
    }

    /// Applique `f` à chaque sélection puis renormalise.
    pub fn map(&self, mut f: impl FnMut(Selection) -> Selection) -> Self {
        let ranges: Vec<_> = self.ranges.iter().map(|&s| f(s)).collect();
        Self::from_vec(ranges, self.primary)
    }

    /// Borne chaque position à `len` (après un rechargement, par exemple).
    pub fn clamp(&self, len: usize) -> Self {
        self.map(|s| Selection {
            anchor: s.anchor.min(len),
            head: s.head.min(len),
            goal_column: s.goal_column,
        })
    }

    fn normalize(&mut self, primary: Selection) {
        self.ranges.sort_by_key(|s| (s.start(), s.end()));
        let mut merged: Vec<Selection> = Vec::with_capacity(self.ranges.len());
        let mut primary_index = 0;
        for selection in self.ranges.drain(..) {
            let is_primary = selection == primary;
            match merged.last_mut() {
                // Deux curseurs au même endroit, ou deux sélections qui se
                // recouvrent : on n'en garde qu'une.
                Some(last)
                    if selection.start() < last.end()
                        || (selection.start() == last.end()
                            && (selection.is_empty() || last.is_empty())) =>
                {
                    let start = last.start().min(selection.start());
                    let end = last.end().max(selection.end());
                    let forward = last.head >= last.anchor;
                    *last = if forward {
                        Selection::new(start, end)
                    } else {
                        Selection::new(end, start)
                    };
                    if is_primary {
                        primary_index = merged.len() - 1;
                    }
                }
                _ => {
                    if is_primary {
                        primary_index = merged.len();
                    }
                    merged.push(selection);
                }
            }
        }
        self.ranges = merged;
        self.primary = primary_index.min(self.ranges.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_are_sorted_and_merged() {
        let selections = Selections::from_vec(
            vec![
                Selection::new(10, 12),
                Selection::cursor(3),
                Selection::new(11, 15),
                Selection::cursor(3),
            ],
            1,
        );
        assert_eq!(selections.len(), 2);
        assert_eq!(selections.as_slice()[0], Selection::cursor(3));
        assert_eq!(selections.as_slice()[1].range(), 10..15);
        assert_eq!(selections.primary(), Selection::cursor(3));
    }
}
