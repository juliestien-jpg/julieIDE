//! Lignes sûres d'un diff, **sur le fichier entier** : celles que tous les
//! alignements les plus courts de `a` et `b` apparient comme notre diff.
//!
//! Compter les alignements optimaux demande en principe une table de
//! `lignes de a × lignes de b` cases : impensable sur 20 000 lignes. Trois
//! remarques la ramènent à de petites bandes autour des changements.
//!
//! 1. **Les lignes sans vis-à-vis possible ne comptent pas.** Une ligne dont le
//!    texte n'existe pas de l'autre côté est changée dans tous les alignements :
//!    on la retire des deux suites. Dans du code, presque toutes les lignes
//!    modifiées sont dans ce cas ; il ne reste que le coût *discrétionnaire* du
//!    diff, celui des lignes banales (vides, accolades, lignes répétées) qu'un
//!    autre alignement pourrait garder.
//! 2. **Un alignement optimal ne s'écarte du nôtre que par détours**, et un
//!    détour ne s'éloigne pas de plus que le coût des changements de notre diff
//!    qu'il enjambe : la table se limite à une bande de cette demi-largeur.
//! 3. **Les lignes uniques cloisonnent.** Une ligne commune dont le texte
//!    n'existe qu'une fois de chaque côté ne peut s'apparier qu'à sa jumelle ;
//!    la sauter coûte une suppression et une insertion. Un détour ne saute donc
//!    pas plus de lignes uniques que la moitié de son coût : passé ce nombre,
//!    tous les alignements optimaux sont revenus sur le nôtre, et le calcul
//!    s'arrête là (voir [`plan`]).
//!
//! Le travail est en `lignes proches d'un changement × coût discrétionnaire
//! local`. Au-delà d'un budget, la fonction renonce (`None`) et la fusion se
//! rabat sur l'analyse par fenêtres, plus grossière.
//!
//! Pourquoi le fichier entier : une analyse par fenêtres suppose que les lignes
//! *entre* deux fenêtres s'apparient comme dans notre diff. C'est faux quand un
//! motif se répète : « insérer deux lignes ici et en supprimer une plus bas »
//! peut aussi se lire « supprimer une ligne ici et en insérer deux plus bas »,
//! tout le milieu étant décalé d'un cran. Seul un décompte qui ne s'arrête
//! qu'aux lignes uniques le voit.

use crate::myers::Change;

/// Module des décomptes de chemins (nombre premier de Mersenne 2^61 - 1) : une
/// ligne déclarée sûre à tort demanderait une coïncidence à 2^-61.
const PRIME: u64 = (1 << 61) - 1;
const NONE: i32 = -1;
const NO_CHANGE: usize = usize::MAX;

/// Résultat de l'analyse globale.
pub(crate) struct Stability {
    /// Pour chaque ligne de `a` : gardée par notre diff et appariée de la même
    /// façon par tous les alignements optimaux.
    pub stable: Vec<bool>,
    /// Coût de notre diff une fois retirées les lignes sans vis-à-vis possible.
    pub discretionary_cost: usize,
}

/// Une ligne de la table, limitée à sa bande `lo..=hi`.
struct Row {
    lo: usize,
    len: Vec<i32>,
    count: Vec<u64>,
}

impl Row {
    fn new() -> Self {
        Row {
            lo: 0,
            len: Vec::new(),
            count: Vec::new(),
        }
    }

    fn reset(&mut self, lo: usize, hi: usize) {
        self.lo = lo;
        self.len.clear();
        self.len.resize(hi - lo + 1, NONE);
        self.count.clear();
        self.count.resize(hi - lo + 1, 0);
    }

    fn get(&self, y: usize) -> (i32, u64) {
        if y < self.lo || y - self.lo >= self.len.len() {
            (NONE, 0)
        } else {
            (self.len[y - self.lo], self.count[y - self.lo])
        }
    }
}

/// Garde le meilleur de deux façons d'arriver à une case, en additionnant les
/// décomptes à égalité.
fn offer(best: &mut (i32, u64), candidate: (i32, u64), gain: i32) {
    if candidate.0 == NONE {
        return;
    }
    let len = candidate.0 + gain;
    if len > best.0 {
        *best = (len, candidate.1);
    } else if len == best.0 {
        best.1 = (best.1 + candidate.1) % PRIME;
    }
}

/// Lignes sûres du diff `changes` de `a` vers `b`. `None` si le travail
/// dépasserait `budget` cases.
pub(crate) fn stable_lines(
    a: &[u32],
    b: &[u32],
    changes: &[Change],
    budget: usize,
) -> Option<Stability> {
    // Vis-à-vis de chaque ligne gardée de `a` selon notre diff.
    let mut partner: Vec<Option<usize>> = vec![None; a.len()];
    {
        let (mut i, mut j) = (0, 0);
        for change in changes {
            while i < change.a.start {
                partner[i] = Some(j);
                i += 1;
                j += 1;
            }
            i = change.a.end;
            j = change.b.end;
        }
        while i < a.len() {
            partner[i] = Some(j);
            i += 1;
            j += 1;
        }
    }

    // 1. Retirer les lignes sans vis-à-vis possible.
    let max_id = a.iter().chain(b).copied().max().unwrap_or(0) as usize;
    let mut count_a = vec![0u32; max_id + 1];
    let mut count_b = vec![0u32; max_id + 1];
    for &id in a {
        count_a[id as usize] += 1;
    }
    for &id in b {
        count_b[id as usize] += 1;
    }
    let (mut ra, mut ra_origin) = (Vec::new(), Vec::new());
    for (i, &id) in a.iter().enumerate() {
        if count_b[id as usize] > 0 {
            ra.push(id);
            ra_origin.push(i);
        }
    }
    let mut rb = Vec::new();
    let mut rb_position = vec![usize::MAX; b.len()];
    for (j, &id) in b.iter().enumerate() {
        if count_a[id as usize] > 0 {
            rb_position[j] = rb.len();
            rb.push(id);
        }
    }
    let (n, m) = (ra.len(), rb.len());
    // Vis-à-vis dans les suites réduites.
    let reduced_partner: Vec<Option<usize>> = ra_origin
        .iter()
        .map(|&i| partner[i].map(|j| rb_position[j]))
        .collect();
    debug_assert!(reduced_partner.iter().flatten().all(|&y| y != usize::MAX));
    let kept = reduced_partner.iter().flatten().count();
    let cost = (n - kept) + (m - kept);

    // Hors des régions à calculer, toute ligne gardée est sûre.
    let mut stable: Vec<bool> = partner.iter().map(Option::is_some).collect();
    if cost == 0 {
        // Suites réduites identiques : un seul alignement optimal, le nôtre.
        return Some(Stability {
            stable,
            discretionary_cost: 0,
        });
    }

    // 2. Où calculer, et sur quelle largeur.
    let unique: Vec<bool> = ra
        .iter()
        .map(|&id| count_a[id as usize] == 1 && count_b[id as usize] == 1)
        .collect();
    let plan = plan(&reduced_partner, &unique, m);
    let mut remaining = budget;
    let mut scratch = Scratch::new(n);
    for &(start, end) in &plan.regions {
        let region = Region {
            start,
            end,
            start_column: if start == 0 {
                0
            } else {
                reduced_partner[start - 1].expect("ancre gardée") + 1
            },
            end_column: if end == n {
                m
            } else {
                reduced_partner[end].expect("ancre gardée")
            },
        };
        let sure = count_region(
            &ra,
            &rb,
            &reduced_partner,
            &plan,
            &region,
            &mut scratch,
            &mut remaining,
        )?;
        for (x, sure) in (region.start..region.end).zip(sure) {
            if reduced_partner[x].is_some() {
                stable[ra_origin[x]] = sure;
            }
        }
    }
    Some(Stability {
        stable,
        discretionary_cost: cost,
    })
}

/// Ce qu'il faut calculer : les régions de la table, et pour chaque rangée la
/// place de notre diff et l'écart qu'un chemin optimal peut y prendre.
struct Plan {
    /// Colonne où notre diff arrive sur la rangée, et celle d'où il la quitte.
    arrive: Vec<usize>,
    leave: Vec<usize>,
    /// Écart maximal, en colonnes, entre un chemin optimal et notre diff.
    reach: Vec<usize>,
    /// Rangées `début..=fin` à calculer, triées, disjointes. Chaque région part
    /// d'un point de notre diff et finit sur un autre, par lesquels passent tous
    /// les alignements optimaux.
    regions: Vec<(usize, usize)>,
}

/// Prépare le calcul.
///
/// Un chemin optimal quitte notre diff en un point et le retrouve en un autre :
/// un *détour*. Entre les deux, il ne coûte pas plus que notre diff (sinon il ne
/// serait pas optimal) et il n'emprunte aucun de ses appariements ; il ne peut
/// donc pas s'en écarter de plus que le coût `c` des changements de notre diff
/// qu'il enjambe.
///
/// Une ligne *unique* (gardée par notre diff, texte présent une seule fois de
/// chaque côté) ne peut s'apparier qu'à sa jumelle. Un détour qui la saute doit
/// la supprimer et réinsérer sa jumelle : coût 2. D'où deux bornes.
///
/// - **Grappes.** Deux changements consécutifs sont *liés* s'il existe une suite
///   de changements qui les contient tous deux et dont le coût paie les lignes
///   uniques qu'elle enjambe. Les changements liés de proche en proche forment
///   une grappe ; aucun détour ne sort de sa grappe. L'écart possible vaut donc
///   le coût de la grappe sur ses rangées, et le plus grand des deux coûts
///   voisins entre deux grappes.
/// - **Ancres.** Autour d'une grappe de coût `c`, un détour saute au plus `c / 2`
///   lignes uniques : la `(c / 2 + 1)`-ième, de chaque côté, est appariée comme
///   dans notre diff par tous les alignements optimaux. La région à calculer va
///   d'une ancre à l'autre ; deux régions qui se rejoignent n'en font qu'une.
fn plan(partner: &[Option<usize>], unique: &[bool], m: usize) -> Plan {
    let n = partner.len();
    // Notre diff, rangée par rangée ; ses changements (coût, première et
    // dernière rangée) ; les lignes uniques de chaque intervalle (l'intervalle
    // `g` précède le changement `g`).
    let mut arrive = vec![0usize; n + 1];
    let mut leave = vec![0usize; n + 1];
    let mut at_row = vec![NO_CHANGE; n + 1];
    let mut costs: Vec<usize> = Vec::new();
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let mut uniques: Vec<usize> = vec![0];
    let mut pending = false;
    let mut column = 0;
    for x in 0..=n {
        let kept = x < n && partner[x].is_some();
        let target = if x == n {
            m
        } else {
            partner[x].unwrap_or(column)
        };
        let edits = (target - column) + usize::from(x < n && !kept);
        arrive[x] = column;
        leave[x] = target;
        if edits > 0 && !pending {
            costs.push(0);
            rows.push((x, x));
            pending = true;
        }
        if pending {
            let current = costs.len() - 1;
            costs[current] += edits;
            rows[current].1 = x;
            at_row[x] = current;
        }
        if kept {
            if pending {
                pending = false;
                uniques.push(0);
            }
            if unique[x] {
                *uniques.last_mut().expect("intervalle en cours") += 1;
            }
            column = target + 1;
        }
    }
    let changes = costs.len();
    let mut reach = vec![0usize; n + 1];
    if changes == 0 {
        return Plan {
            arrive,
            leave,
            reach,
            regions: Vec::new(),
        };
    }

    // Grappes. `gap(g)` : prix des lignes uniques entre les changements g-1 et g.
    let gap = |g: usize| 2 * uniques.get(g).copied().unwrap_or(0) as i64;
    let mut left = vec![0i64; changes];
    let mut right = vec![0i64; changes];
    for k in 0..changes {
        left[k] = costs[k] as i64;
        if k > 0 {
            left[k] += (left[k - 1] - gap(k)).max(0);
        }
    }
    for k in (0..changes).rev() {
        right[k] = costs[k] as i64;
        if k + 1 < changes {
            right[k] += (right[k + 1] - gap(k + 1)).max(0);
        }
    }
    let mut cluster_cost = vec![0usize; changes];
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut first = 0;
    while first < changes {
        let mut last = first + 1;
        while last < changes && left[last - 1] + right[last] >= gap(last) {
            last += 1;
        }
        let total: usize = costs[first..last].iter().sum();
        cluster_cost[first..last].fill(total);

        // Ancres de la grappe.
        let needed = total / 2 + 1;
        let mut start = rows[first].0;
        let mut seen = 0;
        while start > 0 {
            let row = start - 1;
            if partner[row].is_none() {
                break;
            }
            start = row;
            if unique[row] {
                seen += 1;
                if seen == needed {
                    // L'ancre elle-même reste hors de la région.
                    start = row + 1;
                    break;
                }
            }
            if at_row[row] != NO_CHANGE {
                // Ligne qui clôt le changement précédent : au-dessus, c'est ce
                // changement lui-même. Les deux régions n'en feront qu'une.
                break;
            }
        }
        let mut end = rows[last - 1].1;
        let mut seen = 0;
        while end < n {
            // La rangée `end` porte une ligne gardée (celle qui clôt le
            // changement, ou une ligne de l'intervalle).
            if end > rows[last - 1].1 && at_row[end] != NO_CHANGE {
                break;
            }
            if unique[end] {
                seen += 1;
                if seen == needed {
                    break;
                }
            }
            end += 1;
        }
        match regions.last_mut() {
            Some(previous) if start <= previous.1 => previous.1 = previous.1.max(end),
            _ => regions.push((start, end)),
        }
        first = last;
    }

    // Écart par rangée : la grappe du changement qui s'y joue, sinon le plus
    // grand des coûts des grappes voisines.
    let mut before = NO_CHANGE;
    for x in 0..=n {
        if at_row[x] != NO_CHANGE {
            before = at_row[x];
        }
        if before != NO_CHANGE {
            reach[x] = cluster_cost[before];
        }
    }
    let mut after = NO_CHANGE;
    for x in (0..=n).rev() {
        if at_row[x] != NO_CHANGE {
            after = at_row[x];
        }
        if after != NO_CHANGE {
            reach[x] = reach[x].max(cluster_cost[after]);
        }
    }
    Plan {
        arrive,
        leave,
        reach,
        regions,
    }
}

/// Une région de la table : des rangées `start..=end`, du point
/// `(start, start_column)` au point `(end, end_column)`.
struct Region {
    start: usize,
    end: usize,
    start_column: usize,
    end_column: usize,
}

/// Tampons réutilisés d'une région à l'autre.
struct Scratch {
    forward: Vec<(i32, u64)>,
    backward: Vec<(i32, u64)>,
    rows: [Row; 2],
}

impl Scratch {
    fn new(n: usize) -> Self {
        Scratch {
            forward: vec![(NONE, 0); n],
            backward: vec![(NONE, 0); n],
            rows: [Row::new(), Row::new()],
        }
    }
}

/// Compte les alignements optimaux d'une région et rend, pour chacune de ses
/// rangées `start..end`, si la ligne est appariée comme dans notre diff par
/// tous. `None` si le budget est dépassé.
fn count_region(
    ra: &[u32],
    rb: &[u32],
    partner: &[Option<usize>],
    plan: &Plan,
    region: &Region,
    scratch: &mut Scratch,
    budget: &mut usize,
) -> Option<Vec<bool>> {
    let (x0, x1) = (region.start, region.end);
    let (y0, y1) = (region.start_column, region.end_column);
    let lo = |x: usize| plan.arrive[x].saturating_sub(plan.reach[x]).max(y0);
    let hi = |x: usize| (plan.leave[x] + plan.reach[x]).min(y1);
    let mut cells = 0;
    for x in x0..=x1 {
        cells += hi(x) - lo(x) + 1;
    }
    if cells > *budget {
        return None;
    }
    *budget -= cells;

    // Passe avant : meilleur nombre de lignes gardées du début de la région à
    // chaque case, et nombre de chemins qui l'atteignent. On ne retient que
    // les valeurs aux points de notre diff.
    let [previous, current] = &mut scratch.rows;
    let (mut previous, mut current) = (previous, current);
    let mut total = (NONE, 0);
    for x in x0..=x1 {
        let (lo, hi) = (lo(x), hi(x));
        current.reset(lo, hi);
        for y in lo..=hi {
            let mut best = (NONE, 0);
            if x == x0 && y == y0 {
                best = (0, 1);
            }
            if y > lo {
                offer(&mut best, current.get(y - 1), 0);
            }
            if x > x0 {
                offer(&mut best, previous.get(y), 0);
                if y > y0 && ra[x - 1] == rb[y - 1] {
                    offer(&mut best, previous.get(y - 1), 1);
                }
            }
            current.len[y - lo] = best.0;
            current.count[y - lo] = best.1;
        }
        if x == x1 {
            total = current.get(y1);
        } else if let Some(y) = partner[x] {
            scratch.forward[x] = current.get(y);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    // Passe arrière, de la fin de la région vers chaque case.
    let (mut next, mut current) = (previous, current);
    for x in (x0..=x1).rev() {
        let (lo, hi) = (lo(x), hi(x));
        current.reset(lo, hi);
        for y in (lo..=hi).rev() {
            let mut best = (NONE, 0);
            if x == x1 && y == y1 {
                best = (0, 1);
            }
            if y < hi {
                offer(&mut best, current.get(y + 1), 0);
            }
            if x < x1 {
                offer(&mut best, next.get(y), 0);
                if y < y1 && ra[x] == rb[y] {
                    offer(&mut best, next.get(y + 1), 1);
                }
            }
            current.len[y - lo] = best.0;
            current.count[y - lo] = best.1;
        }
        if x > x0 {
            if let Some(y) = partner[x - 1] {
                scratch.backward[x - 1] = current.get(y + 1);
            }
        }
        std::mem::swap(&mut next, &mut current);
    }

    // Une ligne est sûre quand tous les chemins optimaux passent par son
    // appariement.
    let sure = (x0..x1)
        .map(|x| {
            if partner[x].is_none() {
                return false;
            }
            let (before, after) = (scratch.forward[x], scratch.backward[x]);
            if before.0 == NONE || after.0 == NONE {
                return false;
            }
            let through = before.0 + 1 + after.0;
            let paths = (u128::from(before.1) * u128::from(after.1) % u128::from(PRIME)) as u64;
            through == total.0 && paths == total.1
        })
        .collect();
    Some(sure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::myers::{self, tests::Rng};
    use julie_core::CancelToken;

    /// Référence : énumération de tous les alignements optimaux (suites
    /// minuscules), ligne par ligne.
    fn brute_force(a: &[u32], b: &[u32], changes: &[Change]) -> Option<Vec<bool>> {
        fn lcs(a: &[u32], b: &[u32]) -> Vec<Vec<usize>> {
            let mut table = vec![vec![0; b.len() + 1]; a.len() + 1];
            for i in (0..a.len()).rev() {
                for j in (0..b.len()).rev() {
                    table[i][j] = if a[i] == b[j] {
                        table[i + 1][j + 1] + 1
                    } else {
                        table[i + 1][j].max(table[i][j + 1])
                    };
                }
            }
            table
        }
        fn walk(
            a: &[u32],
            b: &[u32],
            table: &[Vec<usize>],
            i: usize,
            j: usize,
            pairs: &mut Vec<(usize, usize)>,
            seen: &mut Vec<Vec<(usize, usize)>>,
        ) {
            if seen.len() > 3000 {
                return;
            }
            if i == a.len() || j == b.len() {
                seen.push(pairs.clone());
                return;
            }
            if a[i] == b[j] && table[i][j] == table[i + 1][j + 1] + 1 {
                pairs.push((i, j));
                walk(a, b, table, i + 1, j + 1, pairs, seen);
                pairs.pop();
            }
            if table[i + 1][j] == table[i][j] {
                walk(a, b, table, i + 1, j, pairs, seen);
            }
            if table[i][j + 1] == table[i][j] {
                walk(a, b, table, i, j + 1, pairs, seen);
            }
        }
        let table = lcs(a, b);
        let mut seen = Vec::new();
        walk(a, b, &table, 0, 0, &mut Vec::new(), &mut seen);
        if seen.len() > 3000 {
            // Trop d'alignements pour cette référence : cas écarté.
            return None;
        }
        let mut stable = vec![false; a.len()];
        let (mut i, mut j) = (0, 0);
        let check = |i: usize, j: usize| seen.iter().all(|pairs| pairs.contains(&(i, j)));
        for change in changes {
            while i < change.a.start {
                stable[i] = check(i, j);
                i += 1;
                j += 1;
            }
            i = change.a.end;
            j = change.b.end;
        }
        while i < a.len() {
            stable[i] = check(i, j);
            i += 1;
            j += 1;
        }
        Some(stable)
    }

    #[test]
    fn agrees_with_the_enumeration_of_all_optimal_alignments() {
        let none = CancelToken::new();
        let mut rng = Rng(0x0bad_5eed_1234_5678);
        let (mut ambiguous, mut checked) = (0, 0);
        for round in 0..8000 {
            // Un tour sur deux : suites plus longues, alphabet plus large, donc
            // des lignes uniques, des grappes et des régions séparées.
            let (max_len, alphabet, max_edits) = if round % 2 == 0 {
                (13, 2 + rng.below(9), 4)
            } else {
                (40, 5 + rng.below(14), 7)
            };
            let a: Vec<u32> = (0..rng.below(max_len))
                .map(|_| rng.below(alphabet) as u32)
                .collect();
            let mut b = a.clone();
            for _ in 0..rng.below(max_edits) {
                let at = rng.below(b.len() + 1);
                match rng.below(3) {
                    0 if at < b.len() => {
                        b.remove(at);
                    }
                    1 if at < b.len() => b[at] = rng.below(alphabet + 2) as u32,
                    _ => b.insert(at, rng.below(alphabet + 2) as u32),
                }
            }
            let changes = myers::diff_ids(&a, &b, &none).unwrap();
            let found = stable_lines(&a, &b, &changes, usize::MAX).unwrap();
            let Some(expected) = brute_force(&a, &b, &changes) else {
                continue;
            };
            checked += 1;
            assert_eq!(found.stable, expected, "tour {round} : {a:?} -> {b:?}");
            if expected.iter().filter(|s| **s).count()
                + changes.iter().map(|c| c.a.len()).sum::<usize>()
                < a.len()
            {
                ambiguous += 1;
            }
        }
        assert!(checked > 7000, "trop de cas écartés : {checked}");
        assert!(
            ambiguous > 200,
            "le tirage doit produire des cas ambigus : {ambiguous}"
        );
    }

    #[test]
    fn a_shifted_repetition_far_from_any_change_is_seen() {
        // Faille de l'analyse par fenêtres : « insérer deux a ici, supprimer un
        // b sept lignes plus bas » se lit aussi « supprimer le b d'ici, insérer
        // un a plus bas, deux fois ». Aucune ligne du milieu n'est sûre.
        // g b a a b a b b b g  ->  g a a b a a b a b b g
        let (g, b_, a_) = (0, 1, 2);
        let base = [g, b_, a_, a_, b_, a_, b_, b_, b_, g];
        let side = [g, a_, a_, b_, a_, a_, b_, a_, b_, b_, g];
        let changes = vec![Change { a: 1..1, b: 1..3 }, Change { a: 8..9, b: 10..10 }];
        let found = stable_lines(&base, &side, &changes, usize::MAX).unwrap();
        assert!(found.stable[0] && found.stable[9]);
        assert!(
            found.stable[1..9].iter().all(|stable| !stable),
            "{:?}",
            found.stable
        );
    }

    #[test]
    fn lines_without_counterpart_cost_nothing() {
        // 2 000 lignes, une sur dix remplacée par une ligne neuve : coût
        // discrétionnaire nul, tout est sûr, sans table.
        let a: Vec<u32> = (0..2000).collect();
        let b: Vec<u32> = (0..2000)
            .map(|i| if i % 10 == 0 { 5000 + i } else { i })
            .collect();
        let changes = myers::diff_ids(&a, &b, &CancelToken::new()).unwrap();
        let found = stable_lines(&a, &b, &changes, 1).unwrap();
        assert_eq!(found.discretionary_cost, 0);
        assert_eq!(found.stable.iter().filter(|s| **s).count(), 1800);
    }

    #[test]
    fn work_stays_near_the_changes() {
        // 20 000 lignes uniques, une ligne banale toutes les cinquante ; une
        // ligne banale retirée ici, une autre ajoutée bien plus loin. Le calcul
        // s'arrête à la première ligne unique de part et d'autre de chaque
        // changement : quelques dizaines de cases, pas des millions.
        const BANAL: u32 = 1_000_000;
        let a: Vec<u32> = (0..20_000)
            .map(|i| if i % 50 == 0 { BANAL } else { i })
            .collect();
        let mut b = a.clone();
        b.remove(5_000);
        b.insert(15_010, BANAL);
        let changes = myers::diff_ids(&a, &b, &CancelToken::new()).unwrap();
        let found = stable_lines(&a, &b, &changes, 100).expect("budget suffisant");
        assert_eq!(found.discretionary_cost, 2);
        assert_eq!(found.stable.iter().filter(|s| **s).count(), 19_999);
        assert!(!found.stable[5_000]);
    }

    #[test]
    fn gives_up_beyond_the_budget() {
        let a: Vec<u32> = (0..200).map(|i| i % 3).collect();
        let b: Vec<u32> = (0..200).map(|i| (i + 1) % 3).collect();
        let changes = myers::diff_ids(&a, &b, &CancelToken::new()).unwrap();
        assert!(stable_lines(&a, &b, &changes, 100).is_none());
        assert!(stable_lines(&a, &b, &changes, usize::MAX).is_some());
    }
}
