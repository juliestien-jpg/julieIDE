//! Cœur du diff : algorithme de Myers en espace linéaire sur des suites d'entiers.
//!
//! Les lignes (ou les mots, ou les caractères) sont d'abord numérotées : deux
//! éléments égaux ont le même numéro. Ce module ne voit que ces numéros.
//!
//! Garde-fous contre le quadratique :
//!
//! - le préfixe et le suffixe communs sont retirés d'abord ;
//! - un élément absent de l'autre côté est forcément un changement : il est
//!   marqué et retiré avant le calcul (deux fichiers sans rien de commun coûtent
//!   donc un temps linéaire) ;
//! - au-delà d'un coût [`max_cost`] dans une recherche de « serpent du milieu »,
//!   on coupe au point le plus avancé au lieu de chercher l'optimum : le diff
//!   reste **exact** (il transforme bien A en B) mais peut ne plus être minimal.

use std::ops::Range;

use julie_core::CancelToken;

/// Un changement : `a` (plage de l'ancien côté) est remplacé par `b` (plage du
/// nouveau). L'une des deux plages peut être vide, jamais les deux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub a: Range<usize>,
    pub b: Range<usize>,
}

const INVALID: isize = -1;

/// Diff de deux suites de numéros. Les changements sont rendus dans l'ordre,
/// séparés par au moins un élément commun.
pub(crate) fn diff_ids(
    a: &[u32],
    b: &[u32],
    cancel: &CancelToken,
) -> julie_core::Result<Vec<Change>> {
    cancel.check()?;
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && a[prefix] == b[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < a.len() - prefix
        && suffix < b.len() - prefix
        && a[a.len() - 1 - suffix] == b[b.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let a_mid = &a[prefix..a.len() - suffix];
    let b_mid = &b[prefix..b.len() - suffix];

    let mut changed_a = vec![false; a_mid.len()];
    let mut changed_b = vec![false; b_mid.len()];

    if a_mid.is_empty() || b_mid.is_empty() {
        changed_a.fill(true);
        changed_b.fill(true);
    } else {
        // Éléments présents d'un seul côté : changés d'office, retirés du calcul.
        let max_id = a_mid.iter().chain(b_mid).copied().max().unwrap_or(0) as usize;
        let mut in_a = vec![false; max_id + 1];
        let mut in_b = vec![false; max_id + 1];
        for &id in a_mid {
            in_a[id as usize] = true;
        }
        for &id in b_mid {
            in_b[id as usize] = true;
        }
        let (kept_a, index_a) = keep_shared(a_mid, &in_b, &mut changed_a);
        let (kept_b, index_b) = keep_shared(b_mid, &in_a, &mut changed_b);

        let mut flags_a = vec![false; kept_a.len()];
        let mut flags_b = vec![false; kept_b.len()];
        Engine::new(&kept_a, &kept_b).run(&mut flags_a, &mut flags_b, cancel)?;
        for (flag, &index) in flags_a.iter().zip(&index_a) {
            changed_a[index] = *flag;
        }
        for (flag, &index) in flags_b.iter().zip(&index_b) {
            changed_b[index] = *flag;
        }
    }

    let mut changes = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < changed_a.len() || j < changed_b.len() {
        let (start_i, start_j) = (i, j);
        while i < changed_a.len() && changed_a[i] {
            i += 1;
        }
        while j < changed_b.len() && changed_b[j] {
            j += 1;
        }
        if i > start_i || j > start_j {
            changes.push(Change {
                a: prefix + start_i..prefix + i,
                b: prefix + start_j..prefix + j,
            });
        } else {
            debug_assert_eq!(a_mid[i], b_mid[j]);
            i += 1;
            j += 1;
        }
    }
    Ok(compact(changes, a, b))
}

fn keep_shared(items: &[u32], in_other: &[bool], changed: &mut [bool]) -> (Vec<u32>, Vec<usize>) {
    let mut kept = Vec::with_capacity(items.len());
    let mut index = Vec::with_capacity(items.len());
    for (i, &id) in items.iter().enumerate() {
        if in_other[id as usize] {
            kept.push(id);
            index.push(i);
        } else {
            changed[i] = true;
        }
    }
    (kept, index)
}

/// Coût maximal d'une recherche de serpent du milieu avant de couper court.
fn max_cost(total: usize) -> isize {
    let mut root = 1usize;
    while root * root < total {
        root += 1;
    }
    root.max(256) as isize
}

struct Engine<'a> {
    a: &'a [u32],
    b: &'a [u32],
    forward: Vec<isize>,
    backward: Vec<isize>,
    offset: isize,
    max_cost: isize,
}

impl<'a> Engine<'a> {
    fn new(a: &'a [u32], b: &'a [u32]) -> Self {
        let total = a.len() + b.len();
        let offset = total + 3;
        Self {
            a,
            b,
            forward: vec![INVALID; 2 * offset + 1],
            backward: vec![INVALID; 2 * offset + 1],
            offset: offset as isize,
            max_cost: max_cost(total),
        }
    }

    fn run(
        &mut self,
        changed_a: &mut [bool],
        changed_b: &mut [bool],
        cancel: &CancelToken,
    ) -> julie_core::Result<()> {
        let (a, b) = (self.a, self.b);
        let mut stack = vec![(0usize, a.len(), 0usize, b.len())];
        while let Some((mut a0, mut a1, mut b0, mut b1)) = stack.pop() {
            while a0 < a1 && b0 < b1 && a[a0] == b[b0] {
                a0 += 1;
                b0 += 1;
            }
            while a0 < a1 && b0 < b1 && a[a1 - 1] == b[b1 - 1] {
                a1 -= 1;
                b1 -= 1;
            }
            if a0 == a1 {
                changed_b[b0..b1].fill(true);
                continue;
            }
            if b0 == b1 {
                changed_a[a0..a1].fill(true);
                continue;
            }
            cancel.check()?;
            let (x0, y0, x1, y1) = self.middle(a0, a1, b0, b1, cancel)?;
            debug_assert!(x0 <= x1 && y0 <= y1);
            debug_assert!((x0, y0) != (a1 - a0, b1 - b0) && (x1, y1) != (0, 0));
            stack.push((a0, a0 + x0, b0, b0 + y0));
            stack.push((a0 + x1, a1, b0 + y1, b1));
        }
        Ok(())
    }

    /// Serpent du milieu de la boîte `[a0, a1) x [b0, b1)`, en coordonnées
    /// relatives à la boîte : `(x0, y0, x1, y1)`. La boîte n'est pas vide et ses
    /// extrémités diffèrent (préfixe et suffixe communs déjà retirés).
    fn middle(
        &mut self,
        a0: usize,
        a1: usize,
        b0: usize,
        b1: usize,
        cancel: &CancelToken,
    ) -> julie_core::Result<(usize, usize, usize, usize)> {
        let (a, b) = (self.a, self.b);
        let n = (a1 - a0) as isize;
        let m = (b1 - b0) as isize;
        let delta = n - m;
        let odd = delta & 1 != 0;
        let off = self.offset;
        let at = |k: isize| (off + k) as usize;

        self.forward[at(1)] = 0;
        self.forward[at(-1)] = INVALID;
        self.backward[at(1)] = 0;
        self.backward[at(-1)] = INVALID;

        let mut d: isize = 0;
        loop {
            if d > 0 {
                self.forward[at(d + 1)] = INVALID;
                self.forward[at(-d - 1)] = INVALID;
                self.backward[at(d + 1)] = INVALID;
                self.backward[at(-d - 1)] = INVALID;
                if d % 64 == 0 {
                    cancel.check()?;
                }
            }

            // Passe avant.
            let mut k = -d;
            while k <= d {
                let down = self.forward[at(k + 1)];
                let right = self.forward[at(k - 1)];
                let mut x = INVALID;
                if down != INVALID && down - k <= m {
                    x = down;
                }
                if right != INVALID && right < n && right + 1 > x {
                    x = right + 1;
                }
                if x == INVALID {
                    self.forward[at(k)] = INVALID;
                    k += 2;
                    continue;
                }
                let mut y = x - k;
                let (xs, ys) = (x, y);
                while x < n && y < m && a[a0 + x as usize] == b[b0 + y as usize] {
                    x += 1;
                    y += 1;
                }
                self.forward[at(k)] = x;
                if odd {
                    let kr = delta - k;
                    if kr > -d && kr < d {
                        let r = self.backward[at(kr)];
                        if r != INVALID && x + r >= n {
                            return Ok((xs as usize, ys as usize, x as usize, y as usize));
                        }
                    }
                }
                k += 2;
            }

            // Passe arrière (coordonnées retournées).
            let mut k = -d;
            while k <= d {
                let down = self.backward[at(k + 1)];
                let right = self.backward[at(k - 1)];
                let mut x = INVALID;
                if down != INVALID && down - k <= m {
                    x = down;
                }
                if right != INVALID && right < n && right + 1 > x {
                    x = right + 1;
                }
                if x == INVALID {
                    self.backward[at(k)] = INVALID;
                    k += 2;
                    continue;
                }
                let mut y = x - k;
                let (xs, ys) = (x, y);
                while x < n && y < m && a[a1 - 1 - x as usize] == b[b1 - 1 - y as usize] {
                    x += 1;
                    y += 1;
                }
                self.backward[at(k)] = x;
                if !odd {
                    let kf = delta - k;
                    if kf >= -d && kf <= d {
                        let f = self.forward[at(kf)];
                        if f != INVALID && f + x >= n {
                            return Ok((
                                (n - x) as usize,
                                (m - y) as usize,
                                (n - xs) as usize,
                                (m - ys) as usize,
                            ));
                        }
                    }
                }
                k += 2;
            }

            if d >= self.max_cost {
                // Trop cher : on coupe au point le plus avancé de la passe avant.
                let mut best = (INVALID, 0isize);
                let mut k = -d;
                while k <= d {
                    let x = self.forward[at(k)];
                    if x != INVALID && (best.0 == INVALID || 2 * x - k > 2 * best.0 - best.1) {
                        best = (x, k);
                    }
                    k += 2;
                }
                let (x, k) = best;
                debug_assert!(x != INVALID);
                let y = x - k;
                return Ok((x as usize, y as usize, x as usize, y as usize));
            }
            d += 1;
        }
    }
}

/// Fait glisser vers le bas les insertions et suppressions pures tant que c'est
/// possible (même convention que Git : un bloc ambigu est montré le plus bas
/// possible), et soude deux changements devenus contigus.
fn compact(changes: Vec<Change>, a: &[u32], b: &[u32]) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::with_capacity(changes.len());
    let mut iter = changes.into_iter().peekable();
    while let Some(mut current) = iter.next() {
        loop {
            let (limit_a, limit_b) = match iter.peek() {
                Some(next) => (next.a.start, next.b.start),
                None => (a.len(), b.len()),
            };
            if current.b.is_empty() {
                while current.a.end < limit_a && a[current.a.start] == a[current.a.end] {
                    current.a = current.a.start + 1..current.a.end + 1;
                    current.b = current.b.start + 1..current.b.end + 1;
                }
            } else if current.a.is_empty() {
                while current.b.end < limit_b && b[current.b.start] == b[current.b.end] {
                    current.a = current.a.start + 1..current.a.end + 1;
                    current.b = current.b.start + 1..current.b.end + 1;
                }
            }
            match iter.peek() {
                Some(next) if next.a.start == current.a.end && next.b.start == current.b.end => {
                    let pure_before = current.a.is_empty() || current.b.is_empty();
                    current = Change {
                        a: current.a.start..next.a.end,
                        b: current.b.start..next.b.end,
                    };
                    iter.next();
                    if !pure_before {
                        break;
                    }
                }
                _ => break,
            }
        }
        out.push(current);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Petit générateur pseudo-aléatoire déterministe pour les tests.
    pub(crate) struct Rng(pub u64);

    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        pub fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }
    }

    fn apply(a: &[u32], b: &[u32], changes: &[Change]) -> Vec<u32> {
        let mut out = Vec::new();
        let mut pos = 0;
        for change in changes {
            out.extend_from_slice(&a[pos..change.a.start]);
            out.extend_from_slice(&b[change.b.clone()]);
            pos = change.a.end;
        }
        out.extend_from_slice(&a[pos..]);
        out
    }

    fn check_well_formed(a: &[u32], b: &[u32], changes: &[Change]) {
        let mut prev: Option<&Change> = None;
        let mut shift: isize = 0;
        for change in changes {
            assert!(!change.a.is_empty() || !change.b.is_empty());
            assert_eq!(change.a.start as isize + shift, change.b.start as isize);
            if let Some(prev) = prev {
                assert!(prev.a.end < change.a.start, "changements non séparés");
            }
            shift += change.b.len() as isize - change.a.len() as isize;
            prev = Some(change);
        }
        assert_eq!(apply(a, b, changes), b);
    }

    fn lcs_len(a: &[u32], b: &[u32]) -> usize {
        let mut row = vec![0usize; b.len() + 1];
        for &x in a {
            let mut diagonal = 0;
            for (j, &y) in b.iter().enumerate() {
                let above = row[j + 1];
                row[j + 1] = if x == y {
                    diagonal + 1
                } else {
                    row[j + 1].max(row[j])
                };
                diagonal = above;
            }
        }
        row[b.len()]
    }

    fn cost(changes: &[Change]) -> usize {
        changes.iter().map(|c| c.a.len() + c.b.len()).sum()
    }

    #[test]
    fn trivial_cases() {
        let none = CancelToken::new();
        assert!(diff_ids(&[], &[], &none).unwrap().is_empty());
        assert!(diff_ids(&[1, 2, 3], &[1, 2, 3], &none).unwrap().is_empty());
        assert_eq!(
            diff_ids(&[], &[1, 2], &none).unwrap(),
            vec![Change { a: 0..0, b: 0..2 }]
        );
        assert_eq!(
            diff_ids(&[1, 2], &[], &none).unwrap(),
            vec![Change { a: 0..2, b: 0..0 }]
        );
        assert_eq!(
            diff_ids(&[1, 2, 3], &[1, 9, 3], &none).unwrap(),
            vec![Change { a: 1..2, b: 1..2 }]
        );
    }

    #[test]
    fn ambiguous_blocks_slide_down() {
        let none = CancelToken::new();
        // Suppression d'un « 1 » parmi trois : montrée sur le dernier.
        assert_eq!(
            diff_ids(&[0, 1, 1, 1, 2], &[0, 1, 1, 2], &none).unwrap(),
            vec![Change { a: 3..4, b: 3..3 }]
        );
        assert_eq!(
            diff_ids(&[0, 1, 1, 2], &[0, 1, 1, 1, 2], &none).unwrap(),
            vec![Change { a: 3..3, b: 3..4 }]
        );
    }

    #[test]
    fn random_diffs_are_exact_and_minimal() {
        let none = CancelToken::new();
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for round in 0..3000 {
            let alphabet = 2 + rng.below(6);
            let len_a = rng.below(30);
            let len_b = rng.below(30);
            let a: Vec<u32> = (0..len_a).map(|_| rng.below(alphabet) as u32).collect();
            let b: Vec<u32> = (0..len_b).map(|_| rng.below(alphabet) as u32).collect();
            let changes = diff_ids(&a, &b, &none).unwrap();
            check_well_formed(&a, &b, &changes);
            let minimal = a.len() + b.len() - 2 * lcs_len(&a, &b);
            assert_eq!(cost(&changes), minimal, "tour {round}: {a:?} -> {b:?}");
        }
    }

    #[test]
    fn expensive_diffs_stay_exact() {
        // Deux longues suites presque sans alignement : la coupe heuristique joue.
        let none = CancelToken::new();
        let mut rng = Rng(42);
        let a: Vec<u32> = (0..6000).map(|_| rng.below(40) as u32).collect();
        let b: Vec<u32> = (0..6000).map(|_| rng.below(40) as u32).collect();
        let changes = diff_ids(&a, &b, &none).unwrap();
        check_well_formed(&a, &b, &changes);
    }

    #[test]
    fn cancellation_is_reported() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let a: Vec<u32> = (0..100).map(|i| i % 7).collect();
        let b: Vec<u32> = (0..100).map(|i| i % 5).collect();
        assert!(matches!(
            diff_ids(&a, &b, &cancel),
            Err(julie_core::Error::Cancelled)
        ));
    }
}
