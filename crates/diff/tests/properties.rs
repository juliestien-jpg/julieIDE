//! Tests de propriété, sur des textes tirés au hasard (tirage déterministe).
//!
//! Les lignes viennent d'un tout petit alphabet : les répétitions, donc les
//! alignements ambigus, sont la règle.

mod common;

use common::{check_invariants, join_lf, join_random, mutate, open, random_lines, Rng};
use julie_core::{CancelToken, TextPosition};
use julie_diff::{
    diff_lines, merge, merge_chunks, ApplyScope, BlockKind, ChunkKind, DiffOptions, MergeSession,
    Side, SideState, TextLines,
};

const ROUNDS: usize = 3000;

fn triple(rng: &mut Rng) -> (Vec<String>, Vec<String>, Vec<String>) {
    let alphabet = 2 + rng.below(10);
    let base = random_lines(rng, 16, alphabet);
    let ours = mutate(rng, &base, alphabet, 4);
    let theirs = mutate(rng, &base, alphabet, 4);
    (base, ours, theirs)
}

fn with_bom(rng: &mut Rng, text: String) -> String {
    if rng.chance(8) {
        format!("\u{feff}{text}")
    } else {
        text
    }
}

/// Fusionner (base, X, base) rend X, à l'octet près : fins de ligne de toute
/// sorte, saut de ligne final ou non, BOM ou non.
#[test]
fn merging_x_with_the_base_gives_x() {
    let cancel = CancelToken::new();
    let mut rng = Rng(0x0001_2345);
    for round in 0..ROUNDS {
        let (base, x, _) = triple(&mut rng);
        let base = join_random(&mut rng, &base);
        let x = join_random(&mut rng, &x);
        let x = with_bom(&mut rng, x);
        let outcome = merge(&base, &x, &base, &cancel).unwrap();
        assert_eq!(outcome.conflicts, 0, "tour {round}: {base:?} / {x:?}");
        assert_eq!(outcome.text, x, "tour {round}: base {base:?}");
    }
}

/// Fusionner (base, X, X) rend X, à l'octet près.
#[test]
fn merging_x_with_x_gives_x() {
    let cancel = CancelToken::new();
    let mut rng = Rng(0x0002_3456);
    for round in 0..ROUNDS {
        let (base, x, _) = triple(&mut rng);
        let base = join_random(&mut rng, &base);
        let x = join_random(&mut rng, &x);
        let x = with_bom(&mut rng, x);
        let outcome = merge(&base, &x, &x, &cancel).unwrap();
        assert_eq!(outcome.conflicts, 0, "tour {round}: {base:?} / {x:?}");
        assert_eq!(outcome.text, x, "tour {round}: base {base:?}");
    }
}

/// Fusionner (base, base, X) rend X sans conflit. À l'octet près quand tout est
/// en LF ; sinon aux fins de ligne près (le résultat suit celles de notre
/// version, ici la base).
#[test]
fn merging_the_base_with_x_gives_x() {
    let cancel = CancelToken::new();
    let mut rng = Rng(0x0003_4567);
    for round in 0..ROUNDS {
        let (base_lines, x_lines, _) = triple(&mut rng);
        let (base, x) = (join_lf(&base_lines), join_lf(&x_lines));
        let outcome = merge(&base, &base, &x, &cancel).unwrap();
        assert_eq!(outcome.conflicts, 0, "tour {round}");
        assert_eq!(outcome.text, x, "tour {round}: base {base:?}");

        let base = join_random(&mut rng, &base_lines);
        let x = join_random(&mut rng, &x_lines);
        let outcome = merge(&base, &base, &x, &cancel).unwrap();
        assert_eq!(outcome.conflicts, 0, "tour {round}");
        let contents = |text: &str| -> Vec<String> {
            TextLines::new(text).iter().map(str::to_owned).collect()
        };
        assert_eq!(contents(&outcome.text), contents(&x), "tour {round}");
        assert_eq!(
            TextLines::new(&outcome.text).has_final_newline(),
            TextLines::new(&x).has_final_newline(),
            "tour {round}: base {base:?}, leur {x:?}"
        );
    }
}

/// Échanger les deux côtés échange gauche et droite, rien d'autre.
#[test]
fn swapping_sides_mirrors_the_classification() {
    let cancel = CancelToken::new();
    let mut rng = Rng(0x0004_5678);
    for round in 0..ROUNDS {
        let (base, ours, theirs) = triple(&mut rng);
        let (base, ours, theirs) = (join_lf(&base), join_lf(&ours), join_lf(&theirs));
        let chunks = |a: &str, b: &str| {
            merge_chunks(
                &TextLines::new(&base),
                &TextLines::new(a),
                &TextLines::new(b),
                &cancel,
            )
            .unwrap()
        };
        let direct = chunks(&ours, &theirs);
        let swapped = chunks(&theirs, &ours);
        assert_eq!(direct.len(), swapped.len(), "tour {round}");
        for (d, s) in direct.iter().zip(&swapped) {
            let mirrored = match s.kind {
                ChunkKind::Left => ChunkKind::Right,
                ChunkKind::Right => ChunkKind::Left,
                other => other,
            };
            assert_eq!(d.kind, mirrored, "tour {round}");
            assert_eq!((&d.base, &d.ours, &d.theirs), (&s.base, &s.theirs, &s.ours));
        }
    }
}

/// Les tronçons couvrent les trois textes, dans l'ordre et sans trou ; un
/// tronçon inchangé porte les mêmes lignes dans les trois ; un changement d'un
/// seul côté laisse l'autre côté égal à la base.
#[test]
fn chunks_are_a_faithful_partition() {
    let cancel = CancelToken::new();
    let mut rng = Rng(0x0005_6789);
    for round in 0..ROUNDS {
        let (base, ours, theirs) = triple(&mut rng);
        let base = join_random(&mut rng, &base);
        let ours = join_random(&mut rng, &ours);
        let theirs = join_random(&mut rng, &theirs);
        let (b, o, t) = (
            TextLines::new(&base),
            TextLines::new(&ours),
            TextLines::new(&theirs),
        );
        let chunks = merge_chunks(&b, &o, &t, &cancel).unwrap();
        let lines = |text: &TextLines<'_>, range: &std::ops::Range<usize>| -> Vec<(String, bool)> {
            range
                .clone()
                .map(|i| {
                    (
                        text.content(i).to_owned(),
                        text.ending(i) != julie_diff::LineEnding::None,
                    )
                })
                .collect()
        };
        let (mut bp, mut op, mut tp) = (0, 0, 0);
        let mut previous_was_change = false;
        for chunk in &chunks {
            assert_eq!(
                (chunk.base.start, chunk.ours.start, chunk.theirs.start),
                (bp, op, tp)
            );
            bp = chunk.base.end;
            op = chunk.ours.end;
            tp = chunk.theirs.end;
            let in_base = lines(&b, &chunk.base);
            let in_ours = lines(&o, &chunk.ours);
            let in_theirs = lines(&t, &chunk.theirs);
            match chunk.kind {
                ChunkKind::Unchanged => {
                    assert!(!chunk.base.is_empty());
                    assert_eq!(in_base, in_ours, "tour {round}");
                    assert_eq!(in_base, in_theirs, "tour {round}");
                }
                ChunkKind::Left => {
                    assert_eq!(in_base, in_theirs, "tour {round}");
                    assert_ne!(in_base, in_ours, "tour {round}");
                }
                ChunkKind::Right => {
                    assert_eq!(in_base, in_ours, "tour {round}");
                    assert_ne!(in_base, in_theirs, "tour {round}");
                }
                ChunkKind::Identical => {
                    assert_eq!(in_ours, in_theirs, "tour {round}");
                    assert_ne!(in_base, in_ours, "tour {round}");
                }
                ChunkKind::Conflict => {
                    assert_ne!(in_ours, in_theirs, "tour {round}");
                }
            }
            // Deux blocs ne se suivent jamais sans ligne commune entre eux.
            let is_change = chunk.kind.is_change();
            assert!(!(is_change && previous_was_change), "tour {round}");
            previous_was_change = is_change;
        }
        assert_eq!((bp, op, tp), (b.len(), o.len(), t.len()));
    }
}

/// Prendre notre côté partout rend notre version ; leur côté partout, la leur.
/// Aucune ligne de l'une ou de l'autre ne se perd.
#[test]
fn taking_one_side_everywhere_gives_that_side() {
    let mut rng = Rng(0x0006_789a);
    for round in 0..ROUNDS {
        let (base, ours, theirs) = triple(&mut rng);
        let (base, ours, theirs) = (join_lf(&base), join_lf(&ours), join_lf(&theirs));
        for side in [Side::Left, Side::Right] {
            let mut session = open(&base, &ours, &theirs);
            for block in session.blocks() {
                if block.state(side) == SideState::NotApplicable {
                    session.ignore(block.id, side.other()).unwrap();
                } else {
                    session.accept(block.id, side).unwrap();
                }
            }
            assert!(session.is_all_resolved());
            let expected = if side == Side::Left { &ours } else { &theirs };
            assert_eq!(&session.result_text(), expected, "tour {round}");
        }
    }
}

/// Le diff exact décrit bien le passage de la gauche à la droite : rejouer les
/// blocs sur la gauche rend la droite, à l'octet près.
#[test]
fn replaying_an_exact_diff_rebuilds_the_right_side() {
    let cancel = CancelToken::new();
    let mut rng = Rng(0x0007_89ab);
    for round in 0..ROUNDS {
        let (left, right, _) = triple(&mut rng);
        let left = join_random(&mut rng, &left);
        let right = join_random(&mut rng, &right);
        let diff = diff_lines(&left, &right, &DiffOptions::exact(), &cancel).unwrap();
        let (l, r) = (TextLines::new(&left), TextLines::new(&right));
        let raw = |text: &TextLines<'_>, range: std::ops::Range<usize>| -> String {
            range
                .map(|i| format!("{}{}", text.content(i), text.ending(i).as_str()))
                .collect()
        };
        let mut rebuilt = String::new();
        for part in diff.alignment() {
            match part.hunk {
                Some(_) => rebuilt.push_str(&raw(&r, part.right)),
                None => {
                    assert_eq!(part.left.len(), part.right.len());
                    rebuilt.push_str(&raw(&l, part.left));
                }
            }
        }
        assert_eq!(rebuilt, right, "tour {round}: gauche {left:?}");
        assert_eq!(diff.identical, left == right);
        assert_eq!(diff.hunks.is_empty(), left == right);

        // Correspondance ligne à ligne cohérente avec les blocs.
        let relaxed = diff_lines(&left, &right, &DiffOptions::default(), &cancel).unwrap();
        for part in relaxed.alignment() {
            if part.hunk.is_none() {
                for (i, j) in part.left.clone().zip(part.right.clone()) {
                    assert_eq!(l.content(i), r.content(j), "tour {round}");
                    assert_eq!(relaxed.left_to_right(i), julie_diff::LineMatch::Same(j));
                    assert_eq!(relaxed.right_to_left(j), julie_diff::LineMatch::Same(i));
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    text: String,
    blocks: Vec<julie_diff::Block>,
    counts: julie_diff::MergeCounts,
}

fn snapshot(session: &MergeSession) -> Snapshot {
    Snapshot {
        text: session.result_text(),
        blocks: session.blocks(),
        counts: session.counts(),
    }
}

/// Une action au hasard ; rend vrai si elle a créé une étape annulable.
fn random_action(rng: &mut Rng, session: &mut MergeSession) -> bool {
    let blocks = session.block_count();
    let lines = session.result_line_count();
    let side = if rng.chance(2) {
        Side::Left
    } else {
        Side::Right
    };
    let steps_before = count_undo(session);
    match rng.below(8) {
        0 | 1 if blocks > 0 => {
            let _ = session.accept(rng.below(blocks), side);
        }
        2 if blocks > 0 => {
            let _ = session.ignore(rng.below(blocks), side);
        }
        3 => {
            let scope = [ApplyScope::All, ApplyScope::Left, ApplyScope::Right][rng.below(3)];
            session.apply_non_conflicting(scope);
        }
        4 if blocks > 0 => {
            let _ = session.accept_both(rng.below(blocks), side);
        }
        5 => {
            let start = rng.below(lines + 1);
            let end = (start + rng.below(3)).min(lines);
            let typed = ["tapé", "", "x", "alpha"];
            let count = rng.below(3);
            let new: Vec<&str> = (0..count).map(|_| typed[rng.below(typed.len())]).collect();
            session.replace_lines(start..end, &new).unwrap();
        }
        _ => {
            let line = rng.below(lines + 1);
            let end_line = (line + rng.below(2)).min(lines);
            let position = TextPosition::new(line as u32, 0);
            let end = TextPosition::new(end_line as u32, 0);
            let text = ["z", "\n", "un\ndeux", ""][rng.below(4)];
            session.edit(position, end, text).unwrap();
        }
    }
    count_undo(session) > steps_before
}

/// Nombre d'étapes annulables (compté sur une copie).
fn count_undo(session: &MergeSession) -> usize {
    let mut copy = session.clone();
    let mut count = 0;
    while copy.undo() {
        count += 1;
    }
    count
}

/// Annuler défait exactement chaque action, compteur et blocs compris ; rétablir
/// la refait exactement ; tout annuler ramène à l'ouverture.
#[test]
fn undo_and_redo_are_exact() {
    let mut rng = Rng(0x0008_9abc);
    for round in 0..600 {
        let (base, ours, theirs) = triple(&mut rng);
        let base = join_random(&mut rng, &base);
        let ours = join_random(&mut rng, &ours);
        let theirs = join_random(&mut rng, &theirs);
        let mut session = open(&base, &ours, &theirs);
        let opening = snapshot(&session);
        let mut history = vec![];
        for _ in 0..12 {
            let before = snapshot(&session);
            if random_action(&mut rng, &mut session) {
                check_invariants(&session);
                let after = snapshot(&session);
                assert!(session.undo());
                assert_eq!(snapshot(&session), before, "tour {round}: annuler");
                check_invariants(&session);
                assert!(session.redo());
                assert_eq!(snapshot(&session), after, "tour {round}: rétablir");
                history.push(after);
            } else {
                assert_eq!(snapshot(&session), before, "tour {round}: action refusée");
            }
        }
        let end = snapshot(&session);
        // Tout annuler, en repassant par chaque état.
        for expected in history.iter().rev().skip(1) {
            assert!(session.undo());
            assert_eq!(&snapshot(&session), expected, "tour {round}");
        }
        if !history.is_empty() {
            assert!(session.undo());
        }
        assert!(!session.undo());
        assert_eq!(
            snapshot(&session),
            opening,
            "tour {round}: retour à l'ouverture"
        );
        assert!(!session.is_touched());
        while session.redo() {}
        assert_eq!(snapshot(&session), end, "tour {round}: tout rétablir");
    }
}

/// « Appliquer tous » ne touche jamais un conflit, et son résultat ne dépend pas
/// de l'ordre : tous d'un coup, ou gauche puis droite, ou droite puis gauche.
#[test]
fn apply_all_is_order_independent_and_leaves_conflicts_alone() {
    let mut rng = Rng(0x0009_abcd);
    for round in 0..ROUNDS {
        let (base, ours, theirs) = triple(&mut rng);
        let (base, ours, theirs) = (join_lf(&base), join_lf(&ours), join_lf(&theirs));
        let mut all = open(&base, &ours, &theirs);
        let conflicts_before: Vec<_> = all
            .blocks()
            .into_iter()
            .filter(|b| b.kind == BlockKind::Conflict)
            .collect();
        all.apply_non_conflicting(ApplyScope::All);
        let mut left_first = open(&base, &ours, &theirs);
        left_first.apply_non_conflicting(ApplyScope::Left);
        left_first.apply_non_conflicting(ApplyScope::Right);
        let mut right_first = open(&base, &ours, &theirs);
        right_first.apply_non_conflicting(ApplyScope::Right);
        right_first.apply_non_conflicting(ApplyScope::Left);
        assert_eq!(all.result_text(), left_first.result_text(), "tour {round}");
        assert_eq!(all.result_text(), right_first.result_text(), "tour {round}");
        assert_eq!(all.counts().changes, 0);
        assert_eq!(all.counts().conflicts, conflicts_before.len());
        for block in all.blocks() {
            if block.kind == BlockKind::Conflict {
                assert_eq!(
                    (block.left, block.right),
                    (SideState::Pending, SideState::Pending)
                );
                // Le texte du conflit au centre est toujours celui de la base.
                let in_result: Vec<&str> =
                    block.result.clone().map(|i| all.result_line(i)).collect();
                let in_base: Vec<&str> =
                    block.base.clone().map(|i| all.base().content(i)).collect();
                assert_eq!(in_result, in_base, "tour {round}");
            }
        }
    }
}
