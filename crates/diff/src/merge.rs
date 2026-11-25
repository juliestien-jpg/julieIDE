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
//! Un diff n'est pas unique. Déplacer une ligne de deux crans, décaler d'un cran
//! un motif répété, retirer une ligne vide parmi trois : chacun de ces
//! changements peut se décrire de plusieurs façons également courtes, et deux
//! outils (le nôtre, celui de Git) ne partent pas forcément de la même. Si le
//! contact entre les deux côtés se décidait sur la seule lecture que notre diff
//! a retenue, une fusion pourrait passer pour propre ici alors que
//! `git merge-file` y voit un conflit.
//!
//! On teste donc le contact sur la **zone d'ambiguïté** de chaque changement
//! (voir [`extend`]) : le changement lui-même, étendu de part et d'autre
//! jusqu'aux premières lignes **sûres**. Les changements du même côté qui
//! partagent une zone sont soudés. Une ligne est sûre :
//!
//! 1. *au sens strict*, quand **tous** les alignements les plus courts des deux
//!    fichiers l'apparient de la même façon. Le décompte se fait sur le fichier
//!    entier, dans une bande étroite autour de notre diff ([`band`]) ;
//! 2. *une fois les alignements rangés*, quand tous les alignements les plus
//!    courts l'apparient de la même façon **après avoir été mis dans la forme
//!    où Git range les siens** ([`canonical`]). Git ne rend jamais un alignement
//!    brut : il glisse chaque changement à une place déterminée (le plus bas
//!    possible, ou en face d'un changement de l'autre fichier). Les lectures
//!    qui ne diffèrent que par un glissement se ramènent donc à une seule, et
//!    ne sont plus un motif de doute. C'est ce qui sépare proprement « une
//!    méthode modifiée, la suivante modifiée et une troisième ajoutée après
//!    une ligne vide » (écart M1 de la recette de la v0).
//!
//! Le second sens n'est employé que là où il est fondé :
//!
//! - entre deux lignes sûres au sens strict (aucun glissement ne peut les
//!   franchir : ranger ce segment seul revient à ranger le fichier entier) ;
//! - si le nombre d'alignements à ranger reste énumérable ;
//! - si le diff du côté est assez petit pour que celui de Git soit certainement
//!   minimal ([`MAX_COST_FOR_CANONICAL`]) : au-delà, Git coupe court par
//!   heuristique et son alignement peut n'être aucun des plus courts.
//!
//! Sinon, seul le sens strict compte. Conséquences qui restent :
//!
//! - nous déclarons parfois un conflit là où Git fusionne : ligne déplacée ou
//!   motif répété dont une lecture touche le changement d'en face ; ligne
//!   retirée d'une série de lignes identiques quand l'autre côté touche à la
//!   série (voir [`zones_of`]) ; gros diffs à place ambiguë ;
//! - deux changements voisins du même côté peuvent former un seul bloc, lignes
//!   communes intermédiaires comprises ;
//! - `tests/git_compare.rs` tire des fusions au hasard dans un alphabet de
//!   quelques lignes (le pire cas pour l'ambiguïté), puis dans du faux code à
//!   méthodes, lignes vides et accolades, et vérifie qu'aucune n'est déclarée
//!   « sans conflit » là où Git voit un conflit. Ce n'est pas une preuve : le
//!   rangement de Git est refait d'après son comportement, pas partagé avec
//!   lui, et son diff n'est pas toujours l'un des plus courts. C'est une
//!   mesure, à relancer à chaque retouche de ce fichier, de `band.rs` ou de
//!   `canonical.rs`.
//!
//! **Repli.** Quand la bande serait trop large (très gros fichier *et* très
//! gros diff de lignes banales), l'analyse stricte se fait par fenêtres autour
//! des changements ([`window_zones`]), comme avant la v1. Elle suppose que les
//! lignes entre deux fenêtres s'apparient comme dans notre diff, ce qu'un
//! motif répété décalé d'un cran peut démentir : c'est une approximation
//! connue, cantonnée à ce repli.
//!
//! Dans le doute, conflit.

use std::borrow::Cow;
use std::collections::HashSet;
use std::ops::Range;

use julie_core::CancelToken;

use crate::band;
use crate::canonical;
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
/// Travail accordé, pour un côté, au décompte global des alignements (cases de
/// la bande, voir [`band`]) ; au-delà, repli sur l'analyse par fenêtres.
const BAND_BUDGET: usize = 24_000_000;
/// Travail total accordé, pour un côté, à la mise en forme canonique des
/// alignements (voir [`canonical::agreed_lines`]) ; au-delà, l'analyse garde sa
/// réponse la plus prudente.
const CANONICAL_BUDGET: usize = 6_000_000;
/// Coût discrétionnaire maximal du diff d'un côté (lignes supprimées et
/// ajoutées, hors lignes sans vis-à-vis possible : Git les écarte aussi avant de
/// chercher) pour que l'on raisonne sur les alignements rangés. Le diff de Git
/// est exact tant qu'une recherche ne dépasse pas un coût de 256 par moitié,
/// soit 512 pour une boîte ; en deçà de 400 pour tout le fichier, il est donc
/// minimal à coup sûr.
const MAX_COST_FOR_CANONICAL: usize = 400;
/// Module des décomptes de chemins (nombre premier de Mersenne 2^61 - 1).
const PRIME: u64 = (1 << 61) - 1;

/// Fenêtre analysée : les changements `lo..hi` et les zones trouvées.
struct Window {
    lo: usize,
    zones: Vec<Sided>,
}

/// Calcule les zones d'ambiguïté des changements du diff `base → side`.
///
/// Les zones rendues sont triées et strictement séparées par au moins une ligne
/// sûre, ce que le regroupement par blocs suppose.
fn extend(changes: Vec<Change>, base: &[u32], side: &[u32]) -> Vec<Sided> {
    let zones = match exact_zones(&changes, base, side) {
        Some(zones) => zones,
        None => window_zones(&changes, base, side),
    };
    // Deux zones qui se touchent malgré tout (fenêtres du repli qui se
    // chevauchent) n'en font qu'une.
    let mut out: Vec<Sided> = Vec::new();
    for mut zone in zones {
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

/// Zones d'après le décompte global des alignements optimaux ([`band`]).
///
/// Les lignes sûres au sens strict découpent le fichier en segments
/// indépendants : tout alignement optimal passe par elles. Un segment qui
/// contient des changements est d'abord une seule zone ; s'il contient aussi des
/// lignes gardées (donc douteuses), on y regarde de plus près en rangeant tous
/// ses alignements à la façon de Git ([`refine_with_canonical_forms`]).
///
/// `None` si le décompte global dépasse son budget.
fn exact_zones(changes: &[Change], base: &[u32], side: &[u32]) -> Option<Vec<Sided>> {
    if changes.is_empty() {
        return Some(Vec::new());
    }
    let stability = band::stable_lines(base, side, changes, BAND_BUDGET)?;
    let stable = stability.stable;
    let mut canonical_budget = if stability.discretionary_cost <= MAX_COST_FOR_CANONICAL {
        CANONICAL_BUDGET
    } else {
        0
    };
    let mut zones = Vec::new();
    let mut first = 0;
    while first < changes.len() {
        // Changements du même segment : aucune ligne sûre ne les sépare.
        let mut last = first + 1;
        while last < changes.len()
            && !stable[changes[last - 1].a.end..changes[last].a.start]
                .iter()
                .any(|stable| *stable)
        {
            last += 1;
        }
        let mut start = changes[first].a.start;
        while start > 0 && !stable[start - 1] {
            start -= 1;
        }
        let mut end = changes[last - 1].a.end;
        while end < base.len() && !stable[end] {
            end += 1;
        }
        let shift_before = changes[first].b.start as isize - changes[first].a.start as isize;
        let shift_after = changes[last - 1].b.end as isize - changes[last - 1].a.end as isize;
        let segment_a = start..end;
        let segment_b = shifted(start, shift_before)..shifted(end, shift_after);
        // Dans le segment, aucune ligne gardée n'est sûre au sens strict.
        let mut local = vec![false; segment_a.len()];
        refine_with_canonical_forms(
            &mut local,
            &base[segment_a.clone()],
            &side[segment_b.clone()],
            &changes[first..last],
            (segment_a.start, segment_b.start),
            &mut canonical_budget,
        );
        zones.extend(zones_of(&changes[first..last], &local, segment_a, base));
        first = last;
    }
    Some(zones)
}

/// Repli de [`exact_zones`] : zones par fenêtres autour des changements.
///
/// Pour chaque groupe de changements voisins, on prend une fenêtre de la base et
/// du côté, et on y cherche les lignes que *tous* les alignements optimaux de
/// **la fenêtre** apparient de la même façon (voir [`stable_lines`]). La zone
/// d'un changement s'étend jusqu'aux lignes sûres qui l'encadrent ; les
/// changements qui partagent une zone sont soudés. Si l'ambiguïté atteint le
/// bord de la fenêtre, la fenêtre grandit (jusqu'à englober les changements
/// voisins).
///
/// Approximation : les lignes laissées entre deux fenêtres sont tenues pour
/// sûres (voir l'en-tête du module).
fn window_zones(changes: &[Change], base: &[u32], side: &[u32]) -> Vec<Sided> {
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
            break zones_of(&changes[lo..hi], &stable, window_a, base);
        };
        windows.push(Window { lo, zones });
        next = hi;
    }
    windows
        .into_iter()
        .flat_map(|window| window.zones)
        .collect()
}

/// Second regard sur un segment compris entre deux lignes sûres au sens strict,
/// et dont les lignes gardées ne le sont pas : parmi tous les alignements
/// optimaux, Git ne peut rendre que ceux qui sont **rangés à sa façon**
/// (changements glissés à leur place canonique, voir [`canonical`]). Une ligne
/// que tous les alignements optimaux, une fois rangés, apparient comme notre
/// diff est donc sûre elle aussi, même si un alignement non rangé la lisait
/// autrement.
///
/// Les bords du segment sont sûrs au sens strict (tous les alignements optimaux
/// du fichier les apparient de la même façon) : aucun glissement ne peut donc
/// les franchir, et ranger le segment seul revient à ranger le fichier entier.
///
/// Si les alignements sont trop nombreux ou le budget épuisé, rien ne change :
/// on garde la réponse stricte.
fn refine_with_canonical_forms(
    stable: &mut [bool],
    a: &[u32],
    b: &[u32],
    changes: &[Change],
    (a_origin, b_origin): (usize, usize),
    budget: &mut usize,
) {
    let expected = canonical::partners_of(changes, a_origin, b_origin, a.len(), b.len());
    let doubtful = expected
        .iter()
        .zip(stable.iter())
        .any(|(partner, stable)| partner.is_some() && !stable);
    if !doubtful {
        return;
    }
    if let Some(agreed) = canonical::agreed_lines(a, b, &expected, budget) {
        for (line, agreed) in agreed.into_iter().enumerate() {
            // Sûre au sens strict implique sûre une fois rangée ; le « ou » ne
            // sert qu'à ne jamais retirer une garantie déjà acquise.
            stable[line] = stable[line] || agreed;
        }
    }
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
///
/// Une **suppression pure** qui peut glisser (une ligne retirée d'une série de
/// lignes identiques, un motif répété retiré une fois) garde en plus toute sa
/// série pour zone, première ligne exceptée : savoir *laquelle* des lignes
/// identiques a disparu n'a pas de réponse, et si l'autre côté touche à l'une
/// d'elles, c'est peut-être à celle-là. Git range la suppression en bas et
/// fusionne ; nous montrons le conflit. (La première ligne de la série est
/// laissée hors zone pour qu'un changement qui ne fait que border la série,
/// sans y toucher, reste séparé.)
fn zones_of(changes: &[Change], stable: &[bool], window: Range<usize>, base: &[u32]) -> Vec<Sided> {
    let mut zones: Vec<Sided> = Vec::new();
    let mut previous_end = window.start;
    for change in changes {
        let mut ext_start = change.a.start;
        while ext_start > window.start && !stable[ext_start - 1 - window.start] {
            ext_start -= 1;
        }
        if change.b.is_empty() {
            let mut slid = 0;
            while change.a.start - slid > previous_end
                && base[change.a.start - slid - 1] == base[change.a.end - slid - 1]
            {
                slid += 1;
            }
            if slid > 0 {
                ext_start = ext_start.min(change.a.start - slid + 1);
            }
        }
        previous_end = change.a.end;
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
pub(crate) fn stable_lines(
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

    // Chaque diff est rangé comme Git range le sien avant d'être analysé.
    let side_changes = |side_ids: &[u32]| -> Result<Vec<Change>, DiffError> {
        let changes = myers::diff_ids(&base_ids, side_ids, cancel)?;
        Ok(canonical::canonical_changes(changes, &base_ids, side_ids))
    };
    let left = extend(side_changes(&ours_ids)?, &base_ids, &ours_ids);
    cancel.check()?;
    let right = extend(side_changes(&theirs_ids)?, &base_ids, &theirs_ids);

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
    fn a_sliding_change_is_judged_where_git_puts_it() {
        // Notre côté supprime un « x » parmi trois : lequel ? Toutes les
        // lectures se rangent sur le dernier. Leur côté modifie la ligne juste
        // avant la série : à deux lignes de là, pas de contact.
        let base = "a\nx\nx\nx\nb\n";
        let ours = "a\nx\nx\nb\n";
        assert_eq!(
            kinds(base, ours, "A\nx\nx\nx\nb\n"),
            vec![ChunkKind::Right, ChunkKind::Left]
        );
        // La ligne juste après la série touche la suppression rangée : conflit.
        assert_eq!(
            kinds(base, ours, "a\nx\nx\nx\nB\n"),
            vec![ChunkKind::Conflict]
        );
        // Un « x » de la série modifié en face : conflit, où qu'il soit.
        for theirs in ["a\nX\nx\nx\nb\n", "a\nx\nX\nx\nb\n", "a\nx\nx\nX\nb\n"] {
            assert_eq!(
                kinds(base, ours, theirs),
                vec![ChunkKind::Conflict],
                "{theirs:?}"
            );
            assert_eq!(
                kinds(base, theirs, ours),
                vec![ChunkKind::Conflict],
                "{theirs:?}"
            );
        }
    }

    #[test]
    fn an_insertion_after_a_brace_is_judged_after_the_brace() {
        // La fonction ajoutée peut se lire « après l'accolade » ou « accolade
        // comprise, un cran plus haut » : rangée, elle est après. L'instruction
        // modifiée en face en est séparée par l'accolade.
        let base = "f() {\n  un();\n}\nfin\n";
        let ours = "f() {\n  un();\n}\ng() {\n  deux();\n}\nfin\n";
        assert_eq!(
            kinds(base, ours, "f() {\n  UN();\n}\nfin\n"),
            vec![ChunkKind::Right, ChunkKind::Left]
        );
        // La ligne qui suit le point d'insertion le touche : conflit.
        assert_eq!(
            kinds(base, ours, "f() {\n  un();\n}\nFIN\n"),
            vec![ChunkKind::Conflict]
        );
        // L'accolade elle-même modifiée en face : conflit.
        assert_eq!(
            kinds(base, ours, "f() {\n  un();\n};\nfin\n"),
            vec![ChunkKind::Conflict]
        );
    }

    /// Écart M1 de la recette de la v0 : la ligne vide qui précède la méthode
    /// modifiée existe aussi devant la méthode ajoutée. Un alignement brut peut
    /// apparier la ligne vide de la base à la seconde ; rangé, il redevient
    /// celui-ci.
    #[test]
    fn a_method_added_after_an_edited_one_does_not_reach_the_method_before() {
        let base = "class A {\n    a() { return 1; }\n\n    b() { return 2; }\n}\n";
        let ours = "class A {\n    a() { return 10; }\n\n    b() { return 2; }\n}\n";
        let theirs =
            "class A {\n    a() { return 1; }\n\n    b() { return 20; }\n\n    c() {}\n}\n";
        assert_eq!(
            kinds(base, ours, theirs),
            vec![ChunkKind::Left, ChunkKind::Right]
        );
        assert_eq!(
            kinds(base, theirs, ours),
            vec![ChunkKind::Right, ChunkKind::Left]
        );
        // Même chose loin du début et de la fin du fichier.
        let wrap = |text: &str| format!("un\ndeux\n\ntrois\n{text}\nquatre\n\ncinq\n");
        assert_eq!(
            kinds(&wrap(base), &wrap(ours), &wrap(theirs)),
            vec![ChunkKind::Left, ChunkKind::Right]
        );
        // Sans la ligne vide entre a() et b(), les deux changements se touchent.
        let tight = |text: &str| text.replacen("\n\n", "\n", 1);
        assert_eq!(
            kinds(&tight(base), &tight(ours), &tight(theirs)),
            vec![ChunkKind::Conflict]
        );
    }

    #[test]
    fn edits_in_a_run_of_identical_lines_are_replacements_in_place() {
        // Recette, cas « lignes répétées » : chaque côté remplace un « x ».
        let base = "x\nx\nx\nx\nx\nx\n";
        assert_eq!(
            kinds(base, "x\nx\nY\nx\nx\nx\n", "x\nx\nx\nx\nZ\nx\n"),
            vec![ChunkKind::Left, ChunkKind::Right]
        );
        // Voisins : conflit.
        assert_eq!(
            kinds(base, "x\nx\nY\nx\nx\nx\n", "x\nx\nx\nZ\nx\nx\n"),
            vec![ChunkKind::Conflict]
        );
    }

    #[test]
    fn a_large_diff_of_ordinary_lines_keeps_the_strict_analysis() {
        // Le même glissement que plus haut, mais notre côté rebat aussi des
        // centaines de lignes banales plus loin : le diff de Git n'est plus
        // garanti minimal, on ne raisonne plus sur les alignements rangés.
        let tail = |step: usize| -> String {
            (0..600)
                .map(|i| format!("mot {}\n", (i * step) % 7))
                .collect()
        };
        let base = format!("a\nx\nx\nx\nb\nfin\n{}", tail(1));
        let ours = format!("a\nx\nx\nb\nfin\n{}", tail(3));
        let theirs = format!("A\nx\nx\nx\nb\nfin\n{}", tail(1));
        assert!(kinds(&base, &ours, &theirs).contains(&ChunkKind::Conflict));
        // Des centaines de lignes réécrites en lignes neuves ne changent rien :
        // elles n'ont pas d'autre lecture possible.
        let fresh =
            |mark: &str| -> String { (0..600).map(|i| format!("ligne {i}{mark}\n")).collect() };
        let base = format!("a\nx\nx\nx\nb\nfin\n{}", fresh(""));
        let ours = format!("a\nx\nx\nb\nfin\n{}", fresh(" changée"));
        let theirs = format!("A\nx\nx\nx\nb\nfin\n{}", fresh(""));
        assert_eq!(
            kinds(&base, &ours, &theirs),
            vec![ChunkKind::Right, ChunkKind::Left, ChunkKind::Left]
        );
    }

    /// Faille trouvée par le tirage contre Git (et présente avant la v1) : notre
    /// côté se lit « deux lignes insérées en haut, une supprimée sept lignes
    /// plus bas » ou « une supprimée en haut, deux insérées plus bas », tout le
    /// milieu décalé d'un cran. Git retient la seconde lecture, qui touche la
    /// ligne modifiée en face. Aucune ligne du milieu n'est sûre : conflit.
    #[test]
    fn a_shifted_repetition_spanning_distant_changes_is_a_conflict() {
        let base = "g\nb\na\na\nb\na\nb\nb\nb\ng\n";
        let ours = "g\na\na\nb\na\na\nb\na\nb\nb\ng\n";
        for line in 1..9 {
            let mut lines: Vec<&str> = base.lines().collect();
            lines[line] = "Z";
            let theirs = format!("{}\n", lines.join("\n"));
            assert_eq!(
                kinds(base, ours, &theirs),
                vec![ChunkKind::Conflict],
                "ligne {line}"
            );
            assert_eq!(
                kinds(base, &theirs, ours),
                vec![ChunkKind::Conflict],
                "ligne {line} (côtés échangés)"
            );
        }
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
