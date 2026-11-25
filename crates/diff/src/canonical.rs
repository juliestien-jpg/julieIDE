//! Forme canonique d'un alignement : où Git range un changement qui peut glisser.
//!
//! Un même changement se décrit souvent de plusieurs façons : la ligne vide
//! retirée d'une série de trois peut être la première ou la dernière, la méthode
//! ajoutée après une accolade peut se lire « accolade comprise, un cran plus
//! haut ». Ces lectures donnent le même texte ; elles ne diffèrent que par la
//! place du changement, donc par ce qu'il touche chez le voisin.
//!
//! Git ne laisse pas ce choix au hasard. Après son diff, il **compacte** : chaque
//! groupe de lignes changées d'un fichier est glissé le plus haut possible, puis
//! le plus bas possible (en avalant les groupes qu'il rencontre), et finit
//!
//! - en face du dernier groupe de l'autre fichier avec lequel il peut s'aligner
//!   (une suppression et un ajout deviennent ainsi un remplacement),
//! - sinon le plus bas possible.
//!
//! Ce module refait ce rangement, d'après le comportement décrit et observé de
//! `git merge-file` (qui n'emploie pas l'heuristique d'indentation de
//! `git diff`). Il sert deux fois dans la fusion :
//!
//! - [`canonical_changes`] range notre propre diff comme Git rangerait le sien ;
//! - [`agreed_lines`] énumère **tous** les alignements les plus courts d'une
//!   fenêtre, range chacun, et dit quelles lignes de la base gardent le même
//!   vis-à-vis dans tous les cas. Ce sont les lignes sur lesquelles Git ne peut
//!   pas avoir lu autre chose que nous, quel que soit l'alignement d'où il est
//!   parti. Au-delà d'un nombre raisonnable d'alignements, la fonction renonce
//!   et la fusion garde l'analyse la plus prudente.
//!
//! La comparaison systématique à `git merge-file` est dans
//! `tests/git_compare.rs` : rien ici n'est tenu pour acquis sans elle.

use crate::myers::Change;

/// Au-delà de ce nombre d'alignements optimaux dans une fenêtre, on renonce.
const MAX_ALIGNMENTS: u32 = 4096;
/// Au-delà de cette taille (lignes de base × lignes du côté), on renonce.
const MAX_CELLS: usize = 60_000;

/// Une suite maximale de lignes changées d'un fichier, `start..end`, éventuellement
/// vide. Il y en a exactement une entre deux lignes gardées consécutives, une
/// avant la première et une après la dernière : les groupes des deux fichiers se
/// correspondent donc un à un, dans l'ordre.
#[derive(Clone, Copy)]
struct Group {
    start: usize,
    end: usize,
}

fn first_group(changed: &[bool]) -> Group {
    let mut end = 0;
    while end < changed.len() && changed[end] {
        end += 1;
    }
    Group { start: 0, end }
}

fn next_group(changed: &[bool], group: &mut Group) -> bool {
    if group.end == changed.len() {
        return false;
    }
    group.start = group.end + 1;
    group.end = group.start;
    while group.end < changed.len() && changed[group.end] {
        group.end += 1;
    }
    true
}

fn previous_group(changed: &[bool], group: &mut Group) -> bool {
    if group.start == 0 {
        return false;
    }
    group.end = group.start - 1;
    group.start = group.end;
    while group.start > 0 && changed[group.start - 1] {
        group.start -= 1;
    }
    true
}

/// Remonte le groupe d'une ligne si la ligne qui le précède est identique à sa
/// dernière ligne ; il avale le groupe qu'il rejoint.
fn slide_up(lines: &[u32], changed: &mut [bool], group: &mut Group) -> bool {
    if group.start > 0 && lines[group.start - 1] == lines[group.end - 1] {
        group.start -= 1;
        changed[group.start] = true;
        group.end -= 1;
        changed[group.end] = false;
        while group.start > 0 && changed[group.start - 1] {
            group.start -= 1;
        }
        true
    } else {
        false
    }
}

/// Descend le groupe d'une ligne si la ligne qui le suit est identique à sa
/// première ligne ; il avale le groupe qu'il rejoint.
fn slide_down(lines: &[u32], changed: &mut [bool], group: &mut Group) -> bool {
    if group.end < lines.len() && lines[group.start] == lines[group.end] {
        changed[group.start] = false;
        group.start += 1;
        changed[group.end] = true;
        group.end += 1;
        while group.end < lines.len() && changed[group.end] {
            group.end += 1;
        }
        true
    } else {
        false
    }
}

/// Range les groupes d'un fichier. Rend `false` si la correspondance des groupes
/// entre les deux fichiers se perd (ne doit pas arriver ; l'appelant renonce).
fn compact_file(lines: &[u32], changed: &mut [bool], other_changed: &[bool]) -> bool {
    let mut group = first_group(changed);
    let mut other = first_group(other_changed);
    loop {
        if group.end > group.start {
            let mut earliest_end;
            // Dernière position (repérée par la fin du groupe) où le groupe
            // faisait face à un groupe non vide de l'autre fichier.
            let mut facing_other: Option<usize>;
            loop {
                let size = group.end - group.start;
                facing_other = None;
                while slide_up(lines, changed, &mut group) {
                    if !previous_group(other_changed, &mut other) {
                        return false;
                    }
                }
                earliest_end = group.end;
                if other.end > other.start {
                    facing_other = Some(group.end);
                }
                while slide_down(lines, changed, &mut group) {
                    if !next_group(other_changed, &mut other) {
                        return false;
                    }
                    if other.end > other.start {
                        facing_other = Some(group.end);
                    }
                }
                if size == group.end - group.start {
                    break;
                }
            }
            if group.end != earliest_end && facing_other.is_some() {
                // Le groupe peut bouger et peut s'aligner : il remonte jusqu'au
                // dernier groupe d'en face rencontré.
                while other.end == other.start {
                    if !slide_up(lines, changed, &mut group)
                        || !previous_group(other_changed, &mut other)
                    {
                        return false;
                    }
                }
            }
        }
        if !next_group(changed, &mut group) {
            return true;
        }
        if !next_group(other_changed, &mut other) {
            return false;
        }
    }
}

/// Range un alignement complet (lignes changées de `a` et de `b`) : d'abord les
/// groupes de `a`, puis ceux de `b`, comme Git.
fn compact(a: &[u32], b: &[u32], changed_a: &mut [bool], changed_b: &mut [bool]) -> bool {
    compact_file(a, changed_a, changed_b) && compact_file(b, changed_b, changed_a)
}

fn flags_of(changes: &[Change], a_len: usize, b_len: usize) -> (Vec<bool>, Vec<bool>) {
    let mut changed_a = vec![false; a_len];
    let mut changed_b = vec![false; b_len];
    for change in changes {
        changed_a[change.a.clone()].fill(true);
        changed_b[change.b.clone()].fill(true);
    }
    (changed_a, changed_b)
}

fn changes_of(changed_a: &[bool], changed_b: &[bool]) -> Vec<Change> {
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
                a: start_i..i,
                b: start_j..j,
            });
        } else {
            i += 1;
            j += 1;
        }
    }
    changes
}

/// Le diff `changes` de `a` vers `b`, rangé comme Git range le sien. Le texte
/// décrit ne change pas : seules bougent les places des changements qui peuvent
/// glisser.
pub(crate) fn canonical_changes(changes: Vec<Change>, a: &[u32], b: &[u32]) -> Vec<Change> {
    if changes.is_empty() {
        return changes;
    }
    let (mut changed_a, mut changed_b) = flags_of(&changes, a.len(), b.len());
    if compact(a, b, &mut changed_a, &mut changed_b) {
        changes_of(&changed_a, &changed_b)
    } else {
        debug_assert!(false, "correspondance des groupes perdue");
        changes
    }
}

/// Pour chaque ligne de `a`, vis-à-vis dans `b` selon les lignes gardées.
fn partners(changed_a: &[bool], changed_b: &[bool]) -> Vec<Option<usize>> {
    let mut out = vec![None; changed_a.len()];
    let mut j = 0;
    for (i, &changed) in changed_a.iter().enumerate() {
        if changed {
            continue;
        }
        while j < changed_b.len() && changed_b[j] {
            j += 1;
        }
        if j == changed_b.len() {
            // Nombres de lignes gardées différents : alignement incohérent.
            return vec![None; changed_a.len()];
        }
        out[i] = Some(j);
        j += 1;
    }
    out
}

/// Vis-à-vis de chaque ligne de `a` selon le diff `changes` (plages relatives
/// aux origines données).
pub(crate) fn partners_of(
    changes: &[Change],
    a_origin: usize,
    b_origin: usize,
    a_len: usize,
    b_len: usize,
) -> Vec<Option<usize>> {
    let mut changed_a = vec![false; a_len];
    let mut changed_b = vec![false; b_len];
    for change in changes {
        changed_a[change.a.start - a_origin..change.a.end - a_origin].fill(true);
        changed_b[change.b.start - b_origin..change.b.end - b_origin].fill(true);
    }
    partners(&changed_a, &changed_b)
}

const NONE: i32 = -1;

/// Tables des alignements optimaux de `a` et `b`, comptés une seule fois chacun.
///
/// Un alignement est un chemin fait de pas « garder » (les deux lignes sont
/// identiques), « supprimer » (ligne de `a`) et « insérer » (ligne de `b`). Pour
/// qu'un même alignement ne soit pas compté selon tous les entrelacements de ses
/// suppressions et de ses insertions, on impose l'ordre : entre deux lignes
/// gardées, les suppressions d'abord. D'où deux états : *libre* (tout est
/// permis) et *en insertion* (plus de suppression avant la prochaine ligne
/// gardée).
struct Paths {
    width: usize,
    /// Meilleur nombre de lignes gardées de `(i, j)` à la fin, état libre.
    free: Vec<i32>,
    /// Idem en insertion ; [`NONE`] si la fin n'est pas atteignable.
    inserting: Vec<i32>,
    /// Nombre d'alignements optimaux depuis `(i, j)`, plafonné.
    free_count: Vec<u32>,
    inserting_count: Vec<u32>,
}

impl Paths {
    fn new(a: &[u32], b: &[u32]) -> Self {
        let (n, m) = (a.len(), b.len());
        let width = m + 1;
        let size = (n + 1) * width;
        let mut paths = Paths {
            width,
            free: vec![0; size],
            inserting: vec![NONE; size],
            free_count: vec![1; size],
            inserting_count: vec![0; size],
        };
        let cap = |x: u32, y: u32| x.saturating_add(y).min(MAX_ALIGNMENTS + 1);
        for i in (0..=n).rev() {
            for j in (0..=m).rev() {
                let at = i * width + j;
                if i == n {
                    // Il ne reste qu'à insérer la fin de `b`.
                    paths.free[at] = 0;
                    paths.inserting[at] = 0;
                    paths.free_count[at] = 1;
                    paths.inserting_count[at] = 1;
                    continue;
                }
                if j == m {
                    // Il ne reste qu'à supprimer la fin de `a`.
                    paths.free[at] = 0;
                    paths.free_count[at] = 1;
                    paths.inserting[at] = NONE;
                    paths.inserting_count[at] = 0;
                    continue;
                }
                let keep = if a[i] == b[j] {
                    paths.free[at + width + 1] + 1
                } else {
                    NONE
                };
                let keep_count = paths.free_count[at + width + 1];
                let insert = paths.inserting[at + 1];
                let insert_count = paths.inserting_count[at + 1];
                let delete = paths.free[at + width];
                let delete_count = paths.free_count[at + width];

                let best = keep.max(insert);
                let mut count = 0;
                if best != NONE {
                    if keep == best {
                        count = cap(count, keep_count);
                    }
                    if insert == best {
                        count = cap(count, insert_count);
                    }
                }
                paths.inserting[at] = best;
                paths.inserting_count[at] = count;

                let best = keep.max(insert).max(delete);
                let mut count = 0;
                if keep == best {
                    count = cap(count, keep_count);
                }
                if insert == best {
                    count = cap(count, insert_count);
                }
                if delete == best {
                    count = cap(count, delete_count);
                }
                paths.free[at] = best;
                paths.free_count[at] = count;
            }
        }
        paths
    }
}

/// Pour chaque ligne de `a` : vrai si **tous** les alignements optimaux de `a`
/// et `b`, une fois rangés comme Git les range, la gardent et lui donnent le
/// vis-à-vis `expected[i]`. Une ligne sans vis-à-vis attendu rend faux.
///
/// Rend `None` — on ne sait pas, l'appelant reste prudent — si la fenêtre est
/// trop grande, s'il y a trop d'alignements ou si le budget de travail est
/// épuisé. `budget` est décompté du travail fait.
pub(crate) fn agreed_lines(
    a: &[u32],
    b: &[u32],
    expected: &[Option<usize>],
    budget: &mut usize,
) -> Option<Vec<bool>> {
    let (n, m) = (a.len(), b.len());
    let cells = (n + 1) * (m + 1);
    if cells > MAX_CELLS || cells > *budget {
        return None;
    }
    *budget -= cells;
    let paths = Paths::new(a, b);
    let total = paths.free_count[0];
    if total > MAX_ALIGNMENTS {
        return None;
    }
    let work = total as usize * (n + m + 1) * 4;
    if work > *budget {
        return None;
    }
    *budget -= work;

    let width = paths.width;
    let mut agreed: Vec<bool> = expected.iter().map(Option::is_some).collect();
    let mut changed_a = vec![false; n];
    let mut changed_b = vec![false; m];
    // Parcours en profondeur des chemins optimaux ; chaque pas fixe le drapeau de
    // la ligne qu'il consomme, donc une feuille a tous ses drapeaux à jour.
    // Cadre : (i, j, en insertion, prochain pas à essayer).
    let mut stack: Vec<(usize, usize, bool, u8)> = vec![(0, 0, false, 0)];
    while let Some(frame) = stack.last_mut() {
        let (i, j, inserting, step) = *frame;
        if i == n && j == m {
            let mut canonical_a = changed_a.clone();
            let mut canonical_b = changed_b.clone();
            if !compact(a, b, &mut canonical_a, &mut canonical_b) {
                return None;
            }
            let found = partners(&canonical_a, &canonical_b);
            for (line, ok) in agreed.iter_mut().enumerate() {
                *ok = *ok && found[line] == expected[line];
            }
            stack.pop();
            continue;
        }
        if step > 2 {
            stack.pop();
            continue;
        }
        frame.3 += 1;
        let at = i * width + j;
        let current = if inserting {
            paths.inserting[at]
        } else {
            paths.free[at]
        };
        match step {
            0 => {
                if i < n && j < m && a[i] == b[j] && paths.free[at + width + 1] + 1 == current {
                    changed_a[i] = false;
                    changed_b[j] = false;
                    stack.push((i + 1, j + 1, false, 0));
                }
            }
            1 => {
                if !inserting && i < n && paths.free[at + width] == current {
                    changed_a[i] = true;
                    stack.push((i + 1, j, false, 0));
                }
            }
            _ => {
                if j < m && paths.inserting[at + 1] == current {
                    changed_b[j] = true;
                    stack.push((i, j + 1, true, 0));
                }
            }
        }
    }
    Some(agreed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical(a: &[u32], b: &[u32], changes: Vec<Change>) -> Vec<Change> {
        canonical_changes(changes, a, b)
    }

    fn change(a: std::ops::Range<usize>, b: std::ops::Range<usize>) -> Change {
        Change { a, b }
    }

    #[test]
    fn a_free_group_ends_as_low_as_it_can() {
        // Une ligne « 1 » retirée parmi trois : rangée sur la dernière.
        let (a, b) = ([0, 1, 1, 1, 2], [0, 1, 1, 2]);
        for at in 1..4 {
            assert_eq!(
                canonical(&a, &b, vec![change(at..at + 1, at..at)]),
                vec![change(3..4, 3..3)]
            );
        }
        // Une ligne ajoutée : idem.
        for at in 1..4 {
            assert_eq!(
                canonical(&b, &a, vec![change(at..at, at..at + 1)]),
                vec![change(3..3, 3..4)]
            );
        }
    }

    #[test]
    fn a_group_that_can_face_the_other_side_becomes_a_replacement() {
        // « x x x x » → « x Y x x » : Y inséré, un x supprimé n'importe où ;
        // rangé, c'est toujours « le deuxième x remplacé par Y ».
        let (a, b) = ([7, 7, 7, 7], [7, 9, 7, 7]);
        for deleted in 0..4 {
            let changes = if deleted < 1 {
                vec![change(0..1, 0..0), change(2..2, 1..2)]
            } else if deleted == 1 {
                vec![change(1..2, 1..2)]
            } else {
                vec![
                    change(1..1, 1..2),
                    change(deleted..deleted + 1, deleted + 1..deleted + 1),
                ]
            };
            assert_eq!(
                canonical(&a, &b, changes),
                vec![change(1..2, 1..2)],
                "{deleted}"
            );
        }
    }

    #[test]
    fn groups_that_meet_are_merged() {
        // a S b → a S B S c : lu « S, B insérés avant S, puis b devient c », le
        // groupe inséré descend d'un cran et rejoint le suivant.
        let (a, b) = ([1, 0, 2, 5], [1, 0, 3, 0, 4, 5]);
        assert_eq!(
            canonical(&a, &b, vec![change(1..1, 1..3), change(2..3, 4..5)]),
            vec![change(2..3, 2..5)]
        );
        assert_eq!(
            canonical(&a, &b, vec![change(2..3, 2..5)]),
            vec![change(2..3, 2..5)]
        );
    }

    #[test]
    fn nothing_moves_without_a_repeated_line() {
        let (a, b) = ([1, 2, 3, 4], [1, 9, 3, 8, 4]);
        let changes = vec![change(1..2, 1..2), change(3..3, 3..4)];
        assert_eq!(canonical(&a, &b, changes.clone()), changes);
    }

    fn agreed(a: &[u32], b: &[u32], changes: &[Change]) -> Option<Vec<bool>> {
        let expected = partners_of(changes, 0, 0, a.len(), b.len());
        let mut budget = usize::MAX;
        agreed_lines(a, b, &expected, &mut budget)
    }

    #[test]
    fn every_reading_of_a_slide_agrees_once_canonical() {
        // Trois lectures de la suppression, une seule forme canonique : toutes
        // les lignes gardées par cette forme sont sûres.
        let (a, b) = ([0, 1, 1, 1, 2], [0, 1, 1, 2]);
        assert_eq!(
            agreed(&a, &b, &[change(3..4, 3..3)]),
            Some(vec![true, true, true, false, true])
        );
        // Jugé sur une autre lecture que la forme canonique : désaccord.
        assert_eq!(
            agreed(&a, &b, &[change(1..2, 1..1)]),
            Some(vec![true, false, false, false, true])
        );
    }

    #[test]
    fn a_moved_line_has_two_readings_that_stay_apart() {
        // a m k b → a k m b : « m descend » ou « k monte », aucun glissement ne
        // ramène l'une à l'autre.
        let (a, b) = ([0, 1, 2, 3], [0, 2, 1, 3]);
        assert_eq!(
            agreed(&a, &b, &[change(1..2, 1..1), change(3..3, 2..3)]),
            Some(vec![true, false, false, true])
        );
    }

    #[test]
    fn the_blank_line_before_an_added_method_is_safe() {
        // Écart M1 de la recette : a S b } → a S B S c }.
        let (a, b) = ([1, 0, 2, 5], [1, 0, 3, 0, 4, 5]);
        assert_eq!(
            agreed(&a, &b, &[change(2..3, 2..5)]),
            Some(vec![true, true, false, true])
        );
    }

    #[test]
    fn too_many_alignments_is_unknown() {
        // Deux suites sans rapport de 14 lignes chacune tirées de deux
        // symboles : des milliers d'alignements optimaux.
        let a: Vec<u32> = (0..40).map(|i| (i * 7 % 3 == 0) as u32).collect();
        let b: Vec<u32> = (0..40).map(|i| (i * 5 % 4 == 0) as u32).collect();
        let expected = vec![None; a.len()];
        let mut budget = usize::MAX;
        assert!(Paths::new(&a, &b).free_count[0] > MAX_ALIGNMENTS);
        assert_eq!(agreed_lines(&a, &b, &expected, &mut budget), None);
        // Budget épuisé : inconnu aussi.
        let mut budget = 3;
        assert_eq!(
            agreed_lines(&[1, 2], &[1, 2], &[Some(0), Some(1)], &mut budget),
            None
        );
    }

    #[test]
    fn alignments_are_counted_once_each() {
        // « x » → « y » : un seul alignement (supprimer puis insérer), pas deux.
        assert_eq!(Paths::new(&[1], &[2]).free_count[0], 1);
        // « x x » → « x » : deux (supprimer le premier ou le second).
        assert_eq!(Paths::new(&[1, 1], &[1]).free_count[0], 2);
        // Identiques : un.
        assert_eq!(Paths::new(&[1, 2, 3], &[1, 2, 3]).free_count[0], 1);
    }
}
