//! Diff à deux côtés, par lignes, avec la correspondance de lignes pour une vue
//! côte à côte.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

use julie_core::CancelToken;

use crate::inline::{self, InlineDiff};
use crate::myers;
use crate::text::{LineEnding, LineEndingStyle, LineTable, TextLines};
use crate::DiffError;

/// Traitement des espaces (espace et tabulation) dans la comparaison des lignes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Whitespace {
    /// Tout compte.
    #[default]
    Exact,
    /// Les espaces en fin de ligne ne comptent pas.
    IgnoreTrailing,
    /// La quantité d'espaces ne compte pas (une suite d'espaces = un espace, fin
    /// de ligne ignorée), mais « ab » reste différent de « a b ».
    IgnoreAmount,
    /// Aucun espace ne compte.
    IgnoreAll,
}

/// Options du diff à deux côtés.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffOptions {
    pub whitespace: Whitespace,
    /// `false` (défaut) : `\r\n` et `\n` sont la même fin de ligne ; la différence
    /// de style est rapportée à part dans [`Diff::left_style`] / [`Diff::right_style`].
    pub line_endings_matter: bool,
    /// `false` (défaut) : l'absence de saut de ligne final n'est pas un bloc ;
    /// elle est rapportée dans [`Diff::final_newline`].
    pub final_newline_matters: bool,
    /// Calculer l'affinage au mot et au caractère dans les blocs modifiés.
    pub inline: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            whitespace: Whitespace::Exact,
            line_endings_matter: false,
            final_newline_matters: false,
            inline: true,
        }
    }
}

impl DiffOptions {
    /// Comparaison à l'octet près (celle qu'il faut pour fabriquer un patch).
    pub fn exact() -> Self {
        Self {
            whitespace: Whitespace::Exact,
            line_endings_matter: true,
            final_newline_matters: true,
            inline: false,
        }
    }
}

/// Nature d'un bloc de diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HunkKind {
    /// Lignes présentes à droite seulement (`left` est vide).
    Added,
    /// Lignes présentes à gauche seulement (`right` est vide).
    Removed,
    /// Lignes remplacées.
    Modified,
}

/// Un bloc de différence : les lignes `left` de gauche correspondent aux lignes
/// `right` de droite. Une plage vide indique un point d'insertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub kind: HunkKind,
    pub left: Range<usize>,
    pub right: Range<usize>,
    /// Affinage au mot/caractère, pour un bloc `Modified` de taille raisonnable
    /// et si [`DiffOptions::inline`] est vrai.
    pub inline: Option<InlineDiff>,
}

/// Évolution du saut de ligne final entre la gauche et la droite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FinalNewlineChange {
    Same,
    /// Absent à gauche, présent à droite.
    Added,
    /// Présent à gauche, absent à droite.
    Removed,
}

/// Tronçon de la correspondance gauche/droite : soit des lignes identiques
/// (plages de même longueur, `hunk` à `None`), soit un bloc de diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignedRange {
    pub left: Range<usize>,
    pub right: Range<usize>,
    /// Indice dans [`Diff::hunks`] si le tronçon est un bloc de différence.
    pub hunk: Option<usize>,
}

/// Où tombe une ligne d'un côté dans l'autre côté.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineMatch {
    /// Ligne identique : son numéro de l'autre côté.
    Same(usize),
    /// Ligne dans un bloc de différence : le bloc et la plage qui lui fait face.
    Changed { hunk: usize, other: Range<usize> },
}

/// Résultat d'un diff à deux côtés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    /// Blocs de différence, dans l'ordre du fichier, séparés par au moins une
    /// ligne identique.
    pub hunks: Vec<Hunk>,
    pub left_line_count: usize,
    pub right_line_count: usize,
    pub left_style: LineEndingStyle,
    pub right_style: LineEndingStyle,
    pub final_newline: FinalNewlineChange,
    /// Les deux textes sont identiques à l'octet près.
    pub identical: bool,
}

impl Diff {
    /// Aucun bloc, mais les textes diffèrent par le style des fins de ligne
    /// (« Seules les fins de ligne diffèrent »).
    pub fn only_line_endings_differ(&self) -> bool {
        self.hunks.is_empty()
            && !self.identical
            && self.left_style != self.right_style
            && self.left_style != LineEndingStyle::None
            && self.right_style != LineEndingStyle::None
    }

    /// Nombre de lignes de gauche supprimées ou remplacées, et de lignes de
    /// droite ajoutées ou remplaçantes.
    pub fn changed_line_counts(&self) -> (usize, usize) {
        self.hunks.iter().fold((0, 0), |(removed, added), hunk| {
            (removed + hunk.left.len(), added + hunk.right.len())
        })
    }

    /// La correspondance complète, du début à la fin des deux textes : de quoi
    /// dessiner les connecteurs et synchroniser le défilement.
    pub fn alignment(&self) -> Vec<AlignedRange> {
        let mut out = Vec::with_capacity(self.hunks.len() * 2 + 1);
        let (mut left, mut right) = (0, 0);
        for (index, hunk) in self.hunks.iter().enumerate() {
            if hunk.left.start > left {
                out.push(AlignedRange {
                    left: left..hunk.left.start,
                    right: right..hunk.right.start,
                    hunk: None,
                });
            }
            out.push(AlignedRange {
                left: hunk.left.clone(),
                right: hunk.right.clone(),
                hunk: Some(index),
            });
            left = hunk.left.end;
            right = hunk.right.end;
        }
        if left < self.left_line_count || right < self.right_line_count {
            out.push(AlignedRange {
                left: left..self.left_line_count,
                right: right..self.right_line_count,
                hunk: None,
            });
        }
        out
    }

    /// Où tombe la ligne `line` de gauche dans le texte de droite. Une ligne au-delà
    /// de la fin est ramenée à la fin.
    pub fn left_to_right(&self, line: usize) -> LineMatch {
        self.map_line(line, |hunk| (&hunk.left, &hunk.right))
    }

    /// Où tombe la ligne `line` de droite dans le texte de gauche.
    pub fn right_to_left(&self, line: usize) -> LineMatch {
        self.map_line(line, |hunk| (&hunk.right, &hunk.left))
    }

    fn map_line(
        &self,
        line: usize,
        sides: impl Fn(&Hunk) -> (&Range<usize>, &Range<usize>),
    ) -> LineMatch {
        // Premier bloc dont la fin (côté source) dépasse la ligne.
        let index = self.hunks.partition_point(|hunk| sides(hunk).0.end <= line);
        if let Some(hunk) = self.hunks.get(index) {
            let (from, to) = sides(hunk);
            if from.start <= line {
                return LineMatch::Changed {
                    hunk: index,
                    other: to.clone(),
                };
            }
        }
        let shift = match index.checked_sub(1).map(|i| sides(&self.hunks[i])) {
            Some((from, to)) => to.end as isize - from.end as isize,
            None => 0,
        };
        LineMatch::Same((line as isize + shift).max(0) as usize)
    }
}

/// Numérote des clés : deux clés égales reçoivent le même numéro.
pub(crate) struct Interner<'a> {
    ids: HashMap<(Cow<'a, str>, u8), u32>,
}

impl<'a> Interner<'a> {
    pub fn new() -> Self {
        Self {
            ids: HashMap::new(),
        }
    }

    pub fn intern(&mut self, key: Cow<'a, str>, tag: u8) -> u32 {
        let next = self.ids.len() as u32;
        *self.ids.entry((key, tag)).or_insert(next)
    }
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

pub(crate) fn normalize(content: &str, whitespace: Whitespace) -> Cow<'_, str> {
    match whitespace {
        Whitespace::Exact => Cow::Borrowed(content),
        Whitespace::IgnoreTrailing => Cow::Borrowed(content.trim_end_matches(is_blank)),
        Whitespace::IgnoreAmount => {
            let trimmed = content.trim_end_matches(is_blank);
            let mut out = String::with_capacity(trimmed.len());
            let mut in_blank = false;
            for c in trimmed.chars() {
                if is_blank(c) {
                    if !in_blank {
                        out.push(' ');
                    }
                    in_blank = true;
                } else {
                    out.push(c);
                    in_blank = false;
                }
            }
            if out == content {
                Cow::Borrowed(content)
            } else {
                Cow::Owned(out)
            }
        }
        Whitespace::IgnoreAll => {
            if content.chars().any(is_blank) {
                Cow::Owned(content.chars().filter(|c| !is_blank(*c)).collect())
            } else {
                Cow::Borrowed(content)
            }
        }
    }
}

fn line_ids<'a>(
    text: &'a str,
    table: &LineTable,
    options: &DiffOptions,
    interner: &mut Interner<'a>,
) -> Vec<u32> {
    (0..table.len())
        .map(|line| {
            let ending = table.ending(line);
            let tag = match ending {
                LineEnding::None if options.final_newline_matters => 0,
                LineEnding::CrLf if options.line_endings_matter => 2,
                _ => 1,
            };
            let key = normalize(table.content(text, line), options.whitespace);
            interner.intern(key, tag)
        })
        .collect()
}

/// Diff par lignes de `left` vers `right`.
///
/// Les numéros de ligne suivent le découpage de [`TextLines`]. Une marque BOM en
/// tête est ignorée dans la comparaison. Seule erreur possible : l'annulation.
pub fn diff_lines(
    left: &str,
    right: &str,
    options: &DiffOptions,
    cancel: &CancelToken,
) -> Result<Diff, DiffError> {
    let left_lines = TextLines::new(left);
    let right_lines = TextLines::new(right);
    let mut interner = Interner::new();
    let left_ids = line_ids(left, left_lines.table(), options, &mut interner);
    let right_ids = line_ids(right, right_lines.table(), options, &mut interner);
    let changes = myers::diff_ids(&left_ids, &right_ids, cancel)?;

    let mut hunks = Vec::with_capacity(changes.len());
    for change in changes {
        let kind = if change.a.is_empty() {
            HunkKind::Added
        } else if change.b.is_empty() {
            HunkKind::Removed
        } else {
            HunkKind::Modified
        };
        let inline = if options.inline && kind == HunkKind::Modified {
            cancel.check()?;
            inline::refine(
                &left_lines,
                change.a.clone(),
                &right_lines,
                change.b.clone(),
                options.whitespace,
                cancel,
            )?
        } else {
            None
        };
        hunks.push(Hunk {
            kind,
            left: change.a,
            right: change.b,
            inline,
        });
    }

    let final_newline = if left_lines.is_empty() || right_lines.is_empty() {
        FinalNewlineChange::Same
    } else {
        match (
            left_lines.has_final_newline(),
            right_lines.has_final_newline(),
        ) {
            (false, true) => FinalNewlineChange::Added,
            (true, false) => FinalNewlineChange::Removed,
            _ => FinalNewlineChange::Same,
        }
    };

    Ok(Diff {
        hunks,
        left_line_count: left_lines.len(),
        right_line_count: right_lines.len(),
        left_style: left_lines.style(),
        right_style: right_lines.style(),
        final_newline,
        identical: left == right,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(left: &str, right: &str) -> Diff {
        diff_lines(left, right, &DiffOptions::default(), &CancelToken::new()).unwrap()
    }

    fn shape(diff: &Diff) -> Vec<(HunkKind, Range<usize>, Range<usize>)> {
        diff.hunks
            .iter()
            .map(|h| (h.kind, h.left.clone(), h.right.clone()))
            .collect()
    }

    #[test]
    fn typed_hunks() {
        let d = diff("a\nb\nc\nd\ne\n", "a\nB\nc\ne\nf\n");
        assert_eq!(
            shape(&d),
            vec![
                (HunkKind::Modified, 1..2, 1..2),
                (HunkKind::Removed, 3..4, 3..3),
                (HunkKind::Added, 5..5, 4..5),
            ]
        );
        assert_eq!(d.changed_line_counts(), (2, 2));
        assert!(!d.identical);
    }

    #[test]
    fn identical_and_empty() {
        let d = diff("a\nb\n", "a\nb\n");
        assert!(d.identical && d.hunks.is_empty());
        assert!(d.alignment().len() == 1);
        let d = diff("", "");
        assert!(d.identical && d.alignment().is_empty());
        let d = diff("", "a\nb\n");
        assert_eq!(shape(&d), vec![(HunkKind::Added, 0..0, 0..2)]);
        assert_eq!(d.final_newline, FinalNewlineChange::Same);
        let d = diff("a\nb\n", "");
        assert_eq!(shape(&d), vec![(HunkKind::Removed, 0..2, 0..0)]);
    }

    #[test]
    fn line_endings_alone_are_reported_apart() {
        let d = diff("a\r\nb\r\n", "a\nb\n");
        assert!(d.hunks.is_empty());
        assert!(!d.identical);
        assert!(d.only_line_endings_differ());
        assert_eq!(
            (d.left_style, d.right_style),
            (LineEndingStyle::CrLf, LineEndingStyle::Lf)
        );

        let exact = diff_lines(
            "a\r\nb\r\n",
            "a\nb\n",
            &DiffOptions::exact(),
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(shape(&exact), vec![(HunkKind::Modified, 0..2, 0..2)]);
    }

    #[test]
    fn final_newline_is_reported_apart() {
        let d = diff("a\nb", "a\nb\n");
        assert!(d.hunks.is_empty());
        assert_eq!(d.final_newline, FinalNewlineChange::Added);
        assert!(!d.only_line_endings_differ());
        let d = diff("a\nb\n", "a\nb");
        assert_eq!(d.final_newline, FinalNewlineChange::Removed);

        let exact =
            diff_lines("a\nb", "a\nb\n", &DiffOptions::exact(), &CancelToken::new()).unwrap();
        assert_eq!(shape(&exact), vec![(HunkKind::Modified, 1..2, 1..2)]);
    }

    #[test]
    fn bom_is_ignored_in_comparison() {
        let d = diff("\u{feff}a\n", "a\n");
        assert!(d.hunks.is_empty());
        assert!(!d.identical);
    }

    #[test]
    fn whitespace_options() {
        let with = |whitespace| {
            let options = DiffOptions {
                whitespace,
                ..DiffOptions::default()
            };
            let d = diff_lines(
                "a  b\nc\nd e\n",
                "a b  \nc\t\nde\n",
                &options,
                &CancelToken::new(),
            )
            .unwrap();
            d.hunks
                .iter()
                .flat_map(|h| h.left.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(with(Whitespace::Exact), vec![0, 1, 2]);
        assert_eq!(with(Whitespace::IgnoreTrailing), vec![0, 2]);
        assert_eq!(with(Whitespace::IgnoreAmount), vec![2]);
        assert_eq!(with(Whitespace::IgnoreAll), Vec::<usize>::new());
    }

    #[test]
    fn alignment_covers_both_sides() {
        let d = diff("a\nb\nc\nd\n", "a\nX\nY\nc\n");
        let alignment = d.alignment();
        assert_eq!(
            alignment,
            vec![
                AlignedRange {
                    left: 0..1,
                    right: 0..1,
                    hunk: None
                },
                AlignedRange {
                    left: 1..2,
                    right: 1..3,
                    hunk: Some(0)
                },
                AlignedRange {
                    left: 2..3,
                    right: 3..4,
                    hunk: None
                },
                AlignedRange {
                    left: 3..4,
                    right: 4..4,
                    hunk: Some(1)
                },
            ]
        );
        assert_eq!(d.left_to_right(0), LineMatch::Same(0));
        assert_eq!(
            d.left_to_right(1),
            LineMatch::Changed {
                hunk: 0,
                other: 1..3
            }
        );
        assert_eq!(d.left_to_right(2), LineMatch::Same(3));
        assert_eq!(
            d.left_to_right(3),
            LineMatch::Changed {
                hunk: 1,
                other: 4..4
            }
        );
        assert_eq!(d.right_to_left(3), LineMatch::Same(2));
        assert_eq!(
            d.right_to_left(2),
            LineMatch::Changed {
                hunk: 0,
                other: 1..2
            }
        );
    }
}
