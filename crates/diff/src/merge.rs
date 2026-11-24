//! Fusion à trois volets : découpage en blocs classés.
//!
//! Règle (voir `docs/specs/fusion.md`) :
//!
//! 1. diff par lignes base→nôtre et base→leur ; `\r\n` et `\n` sont la même fin
//!    de ligne, mais l'absence de saut de ligne final fait partie de la ligne ;
//! 2. deux changements de côtés opposés qui se chevauchent **ou se touchent** dans
//!    la base forment un même bloc, de proche en proche ;
//! 3. un bloc changé d'un seul côté est un changement de ce côté ; changé des
//!    deux côtés avec exactement le même texte, un changement identique ; tout
//!    le reste est un **conflit**.
//!
//! # Zones d'ambiguïté (précaution propre à JulieStorm)
//!
//! Un diff n'est pas unique. Supprimer une ligne vide parmi trois, insérer une
//! fonction entre deux accolades fermantes, déplacer une ligne de deux crans :
//! chacun de ces changements peut se décrire de plusieurs façons également
//! courtes, et deux outils (le nôtre, celui de Git) ne choisissent pas forcément
//! la même. Si le contact entre les deux côtés se décidait sur la seule lecture
//! que notre diff a retenue, une fusion pourrait passer pour propre ici alors que
//! `git merge-file` y voit un conflit.
//!
//! On teste donc le contact sur la **zone d'ambiguïté** de chaque changement
//! (voir [`extend`]) : le changement lui-même, étendu de part et d'autre
//! jusqu'aux premières lignes **sûres**, celles que tous les alignements les
//! plus courts apparient de la même façon. Les changements du même côté qui
//! partagent une zone sont soudés. Conséquences :
//!
//! - nous déclarons parfois un conflit là où Git fusionne (deux changements
//!   opposés à une ligne l'un de l'autre, dont l'un a une place ambiguë) ;
//! - deux changements voisins du même côté peuvent former un seul bloc, lignes
//!   communes intermédiaires comprises ;
//! - `tests/git_compare.rs` tire des fusions au hasard dans un alphabet de
//!   quelques lignes (le pire cas pour l'ambiguïté) et vérifie qu'aucune n'est
//!   déclarée « sans conflit » là où Git voit un conflit. Ce n'est pas une
//!   preuve : Git ne cherche pas toujours l'alignement le plus court, et
//!   l'analyse se fait par fenêtres. C'est une mesure.
//!
//! Dans le doute, conflit.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;

use julie_core::CancelToken;

use crate::diff::Interner;
use crate::myers::{self, Change};
use crate::text::{LineEnding, LineTable, TextLines};
use crate::DiffError;

/// Nature d'un tronçon de la fusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkKind {
    /// Les trois versions sont identiques.
    Unchanged,
    /// Seule notre version (gauche) a changé.
    Left,
    /// Seule leur version (droite) a changé.
    Right,
    /// Les deux côtés ont fait exactement le même changement.
    Identical,
    /// Les deux côtés ont changé, différemment (ou trop près l'un de l'autre).
    Conflict,
}

impl ChunkKind {
    /// Le tronçon est un bloc à traiter (tout sauf `Unchanged`).
    pub fn is_change(self) -> bool {
        self != ChunkKind::Unchanged
    }
}

/// Un tronçon : les mêmes lignes vues dans les trois versions. Les tronçons se
/// suivent sans trou et couvrent les trois textes en entier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub kind: ChunkKind,
    /// Lignes de la base.
    pub base: Range<usize>,
    /// Lignes de notre version (gauche).
    pub ours: Range<usize>,
    /// Lignes de leur version (droite).
    pub theirs: Range<usize>,
}

fn merge_ids<'a>(text: &'a str, table: &LineTable, interner: &mut Interner<'a>) -> Vec<u32> {
    (0..table.len())
        .map(|line| {
            let tag = u8::from(table.ending(line) != LineEnding::None);
            interner.intern(Cow::Borrowed(table.content(text, line)), tag)
        })
        .collect()
}

/// Un changement d'un côté — ou plusieurs changements voisins soudés parce que
/// leur découpage est ambigu — avec la **zone d'ambiguïté** `ext_start..ext_end`
/// qu'il occupe dans la base.
struct Sided {
    change: Change,
    ext_start: usize,
    ext_end: usize,
}

/// Marge de lignes communes prise autour d'un groupe de changements pour
/// l'analyser ; elle double tant que l'ambiguïté atteint le bord.
const MARGIN: usize = 3;
/// Taille maximale (lignes de base × lignes du côté) d'une fenêtre d'analyse.
const MAX_WINDOW_CELLS: usize = 400_000;
/// Travail total accordé à l'analyse d'un côté ; au-delà, on se rabat sur la
/// règle simple (plus prudente), ce qui garde le temps de calcul borné.
const CELL_BUDGET: usize = 12_000_000;
/// Module des décomptes de chemins (nombre premier de Mersenne 2^61 - 1).
const PRIME: u64 = (1 << 61) - 1;

/// Fenêtre analysée : les changements `lo..hi` et les zones trouvées.
struct Window {
    lo: usize,
    zones: Vec<Sided>,
}

/// Regroupe les changements du diff `base → side` et calcule leurs zones
/// d'ambiguïté.
///
/// Un diff n'est pas unique : autour d'un changement, d'autres alignements aussi
/// courts existent souvent (une ligne vide retirée d'une série, une fonction
/// insérée entre deux accolades, une ligne déplacée, un motif répété décalé d'un
/// cran). Git peut en avoir retenu un autre que nous. Pour chaque groupe de
/// changements voisins, on prend une fenêtre de la base et du côté, et on y
/// cherche les lignes **sûres** : celles que *tous* les alignements optimaux de
/// la fenêtre apparient de la même façon (voir [`stable_lines`]). La zone d'un
/// changement s'étend jusqu'aux lignes sûres qui l'encadrent ; les changements
/// qui partagent une zone sont soudés. Si l'ambiguïté atteint le bord de la
/// fenêtre, la fenêtre grandit (jusqu'à englober les changements voisins).
///
/// Les zones rendues sont strictement séparées par au moins une ligne sûre, sauf
/// repli faute de budget (elles peuvent alors se toucher, ce qui les fera
/// simplement entrer dans le même bloc).
fn extend(changes: Vec<Change>, base: &[u32], side: &[u32]) -> Vec<Sided> {
    let mut budget = CELL_BUDGET;
    let mut windows: Vec<Window> = Vec::new();
    let mut next = 0;
    while next < changes.len() {
        let mut lo = next;
        let mut hi = next + 1;
        let (mut top, mut bottom) = (MARGIN, MARGIN);
        let zones = loop {
            // Les changements proches, ou qui peuvent se lire comme un
            // déplacement, s'analysent ensemble.
            while hi < changes.len() && belong_together(&changes[hi - 1], &changes[hi], base, side)
            {
                hi += 1;
            }
            let (first, last) = (&changes[lo], &changes[hi - 1]);
            let gap_before = first.a.start - lo.checked_sub(1).map_or(0, |i| changes[i].a.end);
            let gap_after = changes.get(hi).map_or(base.len(), |c| c.a.start) - last.a.end;
            let (t, b) = (top.min(gap_before), bottom.min(gap_after));
            let window_a = first.a.start - t..last.a.end + b;
            let window_b = first.b.start - t..last.b.end + b;
            let cells = (window_a.len() + 1) * (window_b.len() + 1);
            if cells > MAX_WINDOW_CELLS || cells > budget {
                break vec![fallback_zone(&changes[lo..hi], window_a, base, side)];
            }
            budget -= cells;
            let stable = stable_lines(
                &base[window_a.clone()],
                &side[window_b.clone()],
                &changes[lo..hi],
                window_a.start,
                window_b.start,
            );
            let top_ok = window_a.start == 0 || stable[0];
            let bottom_ok = window_a.end == base.len() || stable[stable.len() - 1];
            if !top_ok {
                if t < gap_before {
                    top *= 2;
                } else {
                    // L'ambiguïté remonte jusqu'au changement précédent : on
                    // reprend sa fenêtre avec celle-ci.
                    lo = windows.pop().expect("fenêtre précédente").lo;
                    top = MARGIN;
                }
                continue;
            }
            if !bottom_ok {
                if b < gap_after {
                    bottom *= 2;
                } else {
                    hi += 1;
                    bottom = MARGIN;
                }
                continue;
            }
            break zones_of(&changes[lo..hi], &stable, window_a);
        };
        windows.push(Window { lo, zones });
        next = hi;
    }
    // Dernière passe : deux zones qui se touchent malgré tout (fenêtres qui se
    // chevauchent, repli) n'en font qu'une. Les zones restent ainsi triées et
    // strictement séparées, ce que le regroupement par blocs suppose.
    let mut out: Vec<Sided> = Vec::new();
    for mut zone in windows.into_iter().flat_map(|window| window.zones) {
        while let Some(previous) = out.last() {
            if zone.ext_start > previous.ext_end {
                break;
            }
            let previous = out.pop().expect("zone précédente");
            zone = Sided {
                change: Change {
                    a: previous.change.a.start..zone.change.a.end,
                    b: previous.change.b.start..zone.change.b.end,
                },
                ext_start: previous.ext_start.min(zone.ext_start),
                ext_end: previous.ext_end.max(zone.ext_end),
            };
        }
        out.push(zone);
    }
    out
}

/// Deux changements consécutifs du même côté s'analysent dans la même fenêtre
/// s'ils sont proches, ou si des lignes supprimées par l'un sont insérées par
/// l'autre sur une distance comparable (un déplacement se lit dans les deux
/// sens).
fn belong_together(previous: &Change, next: &Change, base: &[u32], side: &[u32]) -> bool {
    let gap = next.a.start - previous.a.end;
    if gap <= 2 * MARGIN {
        return true;
    }
    let count = |deleted: &[u32], inserted: &[u32]| -> usize {
        if deleted.len() * inserted.len() > 4096 {
            return 0;
        }
        deleted.iter().filter(|id| inserted.contains(id)).count()
    };
    let moved = count(&base[previous.a.clone()], &side[next.b.clone()])
        + count(&base[next.a.clone()], &side[previous.b.clone()]);
    gap <= moved
}

/// Repli quand une fenêtre est trop grande pour être analysée : une seule zone
/// pour tous ses changements, étendue aux lignes voisines qui ressemblent à une
/// ligne de la fenêtre.
fn fallback_zone(changes: &[Change], window: Range<usize>, base: &[u32], side: &[u32]) -> Sided {
    let (first, last) = (&changes[0], &changes[changes.len() - 1]);
    let change = Change {
        a: first.a.start..last.a.end,
        b: first.b.start..last.b.end,
    };
    let mut seen: HashSet<u32> = base[window.clone()].iter().copied().collect();
    seen.extend(side[change.b.clone()].iter().copied());
    let (mut ext_start, mut ext_end) = (window.start, window.end);
    while ext_start > 0 && seen.contains(&base[ext_start - 1]) {
        ext_start -= 1;
    }
    while ext_end < base.len() && seen.contains(&base[ext_end]) {
        ext_end += 1;
    }
    Sided {
        change,
        ext_start,
        ext_end,
    }
}

/// Les zones d'une fenêtre analysée : chaque changement s'étend jusqu'aux lignes
/// sûres qui l'encadrent ; ceux qui se rejoignent sont soudés.
fn zones_of(changes: &[Change], stable: &[bool], window: Range<usize>) -> Vec<Sided> {
    let mut zones: Vec<Sided> = Vec::new();
    for change in changes {
        let mut ext_start = change.a.start;
        while ext_start > window.start && !stable[ext_start - 1 - window.start] {
            ext_start -= 1;
        }
        let mut ext_end = change.a.end;
        while ext_end < window.end && !stable[ext_end - window.start] {
            ext_end += 1;
        }
        match zones.last_mut() {
            Some(previous) if ext_start <= previous.ext_end => {
                previous.change = Change {
                    a: previous.change.a.start..change.a.end,
                    b: previous.change.b.start..change.b.end,
                };
                previous.ext_end = ext_end;
            }
            _ => zones.push(Sided {
                change: change.clone(),
                ext_start,
                ext_end,
            }),
        }
    }
    zones
}

/// Pour chaque ligne de la fenêtre `a` (base), dit si elle est **sûre** : gardée
/// par notre diff, et appariée à la même ligne de `b` par *tous* les alignements
/// optimaux de la fenêtre.
///
/// On compte les chemins optimaux du graphe d'édition (longueur de la plus
/// longue sous-suite commune et nombre de chemins qui l'atteignent, depuis le
/// début et depuis la fin). Une ligne est sûre quand tous les chemins optimaux
/// passent par son appariement. Les décomptes sont pris modulo un grand nombre
/// premier : une ligne déclarée sûre à tort demanderait une coïncidence à
/// 2^-61.
fn stable_lines(
    a: &[u32],
    b: &[u32],
    changes: &[Change],
    a_origin: usize,
    b_origin: usize,
) -> Vec<bool> {
    let (n, m) = (a.len(), b.len());
    let width = m + 1;
    let at = |i: usize, j: usize| i * width + j;
    let add = |x: u64, y: u64| (x + y) % PRIME;

    let mut forward_len = vec![0u32; (n + 1) * width];
    let mut forward_count = vec![1u64; (n + 1) * width];
    for i in 1..=n {
        for j in 1..=m {
            let up = forward_len[at(i - 1, j)];
            let left = forward_len[at(i, j - 1)];
            let diagonal = if a[i - 1] == b[j - 1] {
                Some(forward_len[at(i - 1, j - 1)] + 1)
            } else {
                None
            };
            let best = up.max(left).max(diagonal.unwrap_or(0));
            let mut count = 0;
            if up == best {
                count = add(count, forward_count[at(i - 1, j)]);
            }
            if left == best {
                count = add(count, forward_count[at(i, j - 1)]);
            }
            if diagonal == Some(best) {
                count = add(count, forward_count[at(i - 1, j - 1)]);
            }
            forward_len[at(i, j)] = best;
            forward_count[at(i, j)] = count;
        }
    }
    let mut backward_len = vec![0u32; (n + 1) * width];
    let mut backward_count = vec![1u64; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let down = backward_len[at(i + 1, j)];
            let right = backward_len[at(i, j + 1)];
            let diagonal = if a[i] == b[j] {
                Some(backward_len[at(i + 1, j + 1)] + 1)
            } else {
                None
            };
            let best = down.max(right).max(diagonal.unwrap_or(0));
            let mut count = 0;
            if down == best {
                count = add(count, backward_count[at(i + 1, j)]);
            }
            if right == best {
                count = add(count, backward_count[at(i, j + 1)]);
            }
            if diagonal == Some(best) {
                count = add(count, backward_count[at(i + 1, j + 1)]);
            }
            backward_len[at(i, j)] = best;
            backward_count[at(i, j)] = count;
        }
    }
    let total_len = forward_len[at(n, m)];
    let total_count = forward_count[at(n, m)];

    let mut stable = vec![false; n];
    // Parcours des lignes gardées par notre diff, entre ses changements.
    let (mut i, mut j) = (0, 0);
    let mut check_run = |i: &mut usize, j: &mut usize, until: usize| {
        while *i < until {
            let through = forward_len[at(*i, *j)] + 1 + backward_len[at(*i + 1, *j + 1)];
            let paths = (u128::from(forward_count[at(*i, *j)])
                * u128::from(backward_count[at(*i + 1, *j + 1)])
                % u128::from(PRIME)) as u64;
            stable[*i] = through == total_len && paths == total_count;
            *i += 1;
            *j += 1;
        }
    };
    for change in changes {
        check_run(&mut i, &mut j, change.a.start - a_origin);
        i = change.a.end - a_origin;
        j = change.b.end - b_origin;
    }
    check_run(&mut i, &mut j, n);
    stable
}

/// Découpe la fusion de `base`, `ours` (gauche) et `theirs` (droite) en tronçons
/// classés. Seule erreur possible : l'annulation.
pub fn merge_chunks(
    base: &TextLines<'_>,
    ours: &TextLines<'_>,
    theirs: &TextLines<'_>,
    cancel: &CancelToken,
) -> Result<Vec<Chunk>, DiffError> {
    let mut interner = Interner::new();
    let base_ids = merge_ids(base.text(), base.table(), &mut interner);
    let ours_ids = merge_ids(ours.text(), ours.table(), &mut interner);
    let theirs_ids = merge_ids(theirs.text(), theirs.table(), &mut interner);

    let left = extend(
        myers::diff_ids(&base_ids, &ours_ids, cancel)?,
        &base_ids,
        &ours_ids,
    );
    cancel.check()?;
    let right = extend(
        myers::diff_ids(&base_ids, &theirs_ids, cancel)?,
        &base_ids,
        &theirs_ids,
    );

    let mut chunks = Vec::new();
    // Position courante dans la base, et décalage de chaque côté par rapport à elle.
    let mut base_pos = 0usize;
    let mut shift_left = 0isize;
    let mut shift_right = 0isize;
    let (mut i, mut j) = (0, 0);

    while i < left.len() || j < right.len() {
        // Ouvre un groupe avec le changement qui commence le plus tôt, puis y
        // ajoute tout ce qui le touche (plages étendues), de proche en proche.
        let mut hull: Option<(usize, usize)> = None;
        let mut reach = 0usize;
        let (mut has_left, mut has_right) = (false, false);
        let mut next_shift_left = shift_left;
        let mut next_shift_right = shift_right;
        loop {
            let next_left = left.get(i).map(|c| c.ext_start);
            let next_right = right.get(j).map(|c| c.ext_start);
            let take_left = match (next_left, next_right) {
                (Some(l), Some(r)) => l <= r,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let candidate = if take_left { &left[i] } else { &right[j] };
            if hull.is_some() && candidate.ext_start > reach {
                break;
            }
            let a = &candidate.change.a;
            hull = Some(match hull {
                Some((start, end)) => (start.min(a.start), end.max(a.end)),
                None => (a.start, a.end),
            });
            reach = reach.max(candidate.ext_end);
            let growth = candidate.change.b.len() as isize - a.len() as isize;
            if take_left {
                has_left = true;
                next_shift_left += growth;
                i += 1;
            } else {
                has_right = true;
                next_shift_right += growth;
                j += 1;
            }
        }
        let Some((start, end)) = hull else { break };

        if start > base_pos {
            chunks.push(Chunk {
                kind: ChunkKind::Unchanged,
                base: base_pos..start,
                ours: shifted(base_pos, shift_left)..shifted(start, shift_left),
                theirs: shifted(base_pos, shift_right)..shifted(start, shift_right),
            });
        }
        let ours_range = shifted(start, shift_left)..shifted(end, next_shift_left);
        let theirs_range = shifted(start, shift_right)..shifted(end, next_shift_right);
        let kind = match (has_left, has_right) {
            (true, false) => ChunkKind::Left,
            (false, true) => ChunkKind::Right,
            _ => {
                if ours_ids[ours_range.clone()] == theirs_ids[theirs_range.clone()] {
                    ChunkKind::Identical
                } else {
                    ChunkKind::Conflict
                }
            }
        };
        chunks.push(Chunk {
            kind,
            base: start..end,
            ours: ours_range,
            theirs: theirs_range,
        });
        base_pos = end;
        shift_left = next_shift_left;
        shift_right = next_shift_right;
    }

    if base_pos < base_ids.len() {
        chunks.push(Chunk {
            kind: ChunkKind::Unchanged,
            base: base_pos..base_ids.len(),
            ours: shifted(base_pos, shift_left)..ours_ids.len(),
            theirs: shifted(base_pos, shift_right)..theirs_ids.len(),
        });
    }
    debug_assert!(covers(
        &chunks,
        base_ids.len(),
        ours_ids.len(),
        theirs_ids.len()
    ));
    Ok(chunks)
}

fn shifted(position: usize, shift: isize) -> usize {
    (position as isize + shift) as usize
}

fn covers(chunks: &[Chunk], base: usize, ours: usize, theirs: usize) -> bool {
    let (mut b, mut o, mut t) = (0, 0, 0);
    for chunk in chunks {
        if chunk.base.start != b || chunk.ours.start != o || chunk.theirs.start != t {
            return false;
        }
        b = chunk.base.end;
        o = chunk.ours.end;
        t = chunk.theirs.end;
    }
    b == base && o == ours && t == theirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(base: &str, ours: &str, theirs: &str) -> Vec<ChunkKind> {
        let chunks = merge_chunks(
            &TextLines::new(base),
            &TextLines::new(ours),
            &TextLines::new(theirs),
            &CancelToken::new(),
        )
        .unwrap();
        chunks
            .into_iter()
            .map(|c| c.kind)
            .filter(|k| k.is_change())
            .collect()
    }

    #[test]
    fn separated_changes_stay_apart() {
        assert_eq!(
            kinds("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n"),
            vec![ChunkKind::Left, ChunkKind::Right]
        );
    }

    #[test]
    fn adjacent_changes_conflict() {
        assert_eq!(
            kinds("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n"),
            vec![ChunkKind::Conflict]
        );
    }

    #[test]
    fn chunks_cover_everything_in_order() {
        let base = TextLines::new("a\nb\nc\nd\ne\nf\n");
        let ours = TextLines::new("a\nB\nB2\nc\nd\ne\nf\n");
        let theirs = TextLines::new("a\nb\nc\nd\nf\n");
        let chunks = merge_chunks(&base, &ours, &theirs, &CancelToken::new()).unwrap();
        assert_eq!(
            chunks,
            vec![
                Chunk {
                    kind: ChunkKind::Unchanged,
                    base: 0..1,
                    ours: 0..1,
                    theirs: 0..1
                },
                Chunk {
                    kind: ChunkKind::Left,
                    base: 1..2,
                    ours: 1..3,
                    theirs: 1..2
                },
                Chunk {
                    kind: ChunkKind::Unchanged,
                    base: 2..4,
                    ours: 3..5,
                    theirs: 2..4
                },
                Chunk {
                    kind: ChunkKind::Right,
                    base: 4..5,
                    ours: 5..6,
                    theirs: 4..4
                },
                Chunk {
                    kind: ChunkKind::Unchanged,
                    base: 5..6,
                    ours: 6..7,
                    theirs: 4..5
                },
            ]
        );
    }

    #[test]
    fn sliding_change_conflicts_with_a_neighbour_in_its_range() {
        // Notre côté supprime un « x » parmi trois : lequel ? N'importe. Leur
        // côté modifie la ligne juste avant la série : à une ligne commune de la
        // suppression si on la place en bas, contre elle si on la place en haut.
        let base = "a\nx\nx\nx\nb\n";
        let ours = "a\nx\nx\nb\n";
        let theirs = "A\nx\nx\nx\nb\n";
        assert_eq!(kinds(base, ours, theirs), vec![ChunkKind::Conflict]);
        // Assez loin de toute la plage de glissement : deux changements propres.
        let base = "a\nk\nx\nx\nx\nk\nb\n";
        let ours = "a\nk\nx\nx\nk\nb\n";
        let theirs = "A\nk\nx\nx\nx\nk\nb\n";
        assert_eq!(
            kinds(base, ours, theirs),
            vec![ChunkKind::Right, ChunkKind::Left]
        );
    }

    #[test]
    fn an_insertion_that_could_sit_one_line_higher_reaches_that_line() {
        // La fonction ajoutée peut se lire « après l'accolade » ou « avant » :
        // la ligne « } » fait partie de sa zone d'ambiguïté.
        let base = "f() {\n  un();\n}\nfin\n";
        let ours = "f() {\n  un();\n}\ng() {\n  deux();\n}\nfin\n";
        let theirs = "f() {\n  UN();\n}\nfin\n";
        assert_eq!(kinds(base, ours, theirs), vec![ChunkKind::Conflict]);
        // Une ligne sûre entre les deux suffit à les séparer.
        let base = "f() {\n  un();\n  zéro();\n}\nfin\n";
        let ours = "f() {\n  un();\n  zéro();\n}\ng() {\n  deux();\n}\nfin\n";
        let theirs = "f() {\n  UN();\n  zéro();\n}\nfin\n";
        assert_eq!(
            kinds(base, ours, theirs),
            vec![ChunkKind::Right, ChunkKind::Left]
        );
    }

    #[test]
    fn a_moved_line_is_one_block_with_the_lines_it_jumps_over() {
        // « m » descend sous « k » : est-ce m qui descend ou k qui monte ?
        let base = TextLines::new("a\nm\nk\nb\n");
        let ours = TextLines::new("a\nk\nm\nb\n");
        let chunks = merge_chunks(&base, &ours, &base, &CancelToken::new()).unwrap();
        let blocks: Vec<&Chunk> = chunks.iter().filter(|c| c.kind.is_change()).collect();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, ChunkKind::Left);
        assert_eq!(
            (blocks[0].base.clone(), blocks[0].ours.clone()),
            (1..3, 1..3)
        );
        // L'autre côté touche à la ligne enjambée : conflit.
        assert_eq!(
            kinds("a\nm\nk\nb\n", "a\nk\nm\nb\n", "a\nm\nK\nb\n"),
            vec![ChunkKind::Conflict]
        );
    }

    /// Cas trouvés par le tirage aléatoire contre `git merge-file` : Git y voit
    /// un conflit, parce que son diff lit le changement de notre côté autrement.
    #[test]
    fn alignments_git_reads_differently_are_conflicts() {
        let cases = [
            (
                "alpha\nalpha\nalpha\nbeta\ngamma\nbeta\n",
                "alpha\ngamma\nalpha\nalpha\nbeta\nbeta\ngamma\n",
                "alpha\nalpha\nalpha\nbeta\ngamma\n",
            ),
            (
                "beta\n}\nbeta\n\n",
                "}\nbeta\nbeta\nbeta\n\n",
                "beta\n}\nbeta\n",
            ),
            (
                "gamma\nbeta\n    return;\nbeta\nbeta\n\n\n}\n",
                "gamma\nbeta\n    return;\nbeta\n    return;\n\n}\n",
                "gamma\n\n    return;\nbeta\nbeta\nbeta\n\n\ngamma\n",
            ),
            // Motif alterné « delta alpha delta » : décalable d'un cran.
            (
                "}\ndelta\nalpha\ndelta\ndelta\n",
                "}\ndelta\n}\nalpha\ndelta\ndelta\n",
                "}\nalpha\ndelta\nalpha\nbeta\ndelta\n",
            ),
        ];
        for (base, ours, theirs) in cases {
            assert!(
                kinds(base, ours, theirs).contains(&ChunkKind::Conflict),
                "{base:?}"
            );
            assert!(
                kinds(base, theirs, ours).contains(&ChunkKind::Conflict),
                "{base:?} (côtés échangés)"
            );
        }
    }

    #[test]
    fn same_insertion_on_both_sides_is_identical() {
        assert_eq!(
            kinds("a\nb\n", "a\nX\nb\n", "a\nX\nb\n"),
            vec![ChunkKind::Identical]
        );
        assert_eq!(
            kinds("a\nb\n", "a\nX\nb\n", "a\nY\nb\n"),
            vec![ChunkKind::Conflict]
        );
    }

    #[test]
    fn line_ending_conversion_alone_is_not_a_change() {
        assert!(kinds("a\nb\n", "a\r\nb\r\n", "a\nb\n").is_empty());
        // Mais le saut de ligne final compte.
        assert_eq!(kinds("a\nb", "a\nb\n", "a\nb"), vec![ChunkKind::Left]);
    }
}
