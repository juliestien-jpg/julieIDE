//! Les 26 cas d'essai de `docs/specs/fusion.md`, octet pour octet.
//!
//! « G/D/I/C » = nombre de blocs changement de gauche / de droite / identique /
//! conflit. « Tous » = résultat après « appliquer les changements non
//! conflictuels → tous ».

mod common;

use common::{check_invariants, open, tally};
use julie_diff::{ApplyScope, MergeCounts, Side};

/// Ouvre, vérifie le classement G/D/I/C, vérifie que rien n'est appliqué à
/// l'ouverture, applique « tous » et rend le résultat.
fn all(base: &str, ours: &str, theirs: &str, expected: (usize, usize, usize, usize)) -> String {
    let mut session = open(base, ours, theirs);
    assert_eq!(tally(&session), expected, "classement G/D/I/C");
    assert!(!session.is_touched());
    let counts = session.counts();
    assert_eq!(counts.changes, expected.0 + expected.1 + expected.2);
    assert_eq!(counts.conflicts, expected.3);
    check_invariants(&session);

    session.apply_non_conflicting(ApplyScope::All);
    check_invariants(&session);
    assert_eq!(session.counts().changes, 0);
    assert_eq!(session.counts().conflicts, expected.3);
    session.result_text()
}

/// Le centre à l'ouverture.
fn opening(base: &str, ours: &str, theirs: &str) -> String {
    open(base, ours, theirs).result_text()
}

#[test]
fn case_01_left_change() {
    let (base, ours) = ("a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(opening(base, ours, base), base, "le centre part de la base");
    assert_eq!(all(base, ours, base, (1, 0, 0, 0)), "a\nB\nc\n");
}

#[test]
fn case_02_right_change() {
    let (base, theirs) = ("a\nb\nc\n", "a\nb\nC\n");
    assert_eq!(all(base, base, theirs, (0, 1, 0, 0)), "a\nb\nC\n");
}

#[test]
fn case_03_one_change_each_side() {
    let (base, ours, theirs) = ("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n");
    assert_eq!(all(base, ours, theirs, (1, 1, 0, 0)), "a\nB\nc\nD\ne\n");

    let mut session = open(base, ours, theirs);
    assert_eq!(session.apply_non_conflicting(ApplyScope::Left), 1);
    assert_eq!(session.result_text(), "a\nB\nc\nd\ne\n");
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 1,
            conflicts: 0
        }
    );

    let mut session = open(base, ours, theirs);
    assert_eq!(session.apply_non_conflicting(ApplyScope::Right), 1);
    assert_eq!(session.result_text(), "a\nb\nc\nD\ne\n");
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 1,
            conflicts: 0
        }
    );
}

#[test]
fn case_04_identical_change() {
    let (base, both) = ("a\nb\nc\n", "a\nX\nc\n");
    assert_eq!(all(base, both, both, (0, 0, 1, 0)), "a\nX\nc\n");
}

#[test]
fn case_05_conflict() {
    let mut session = open("a\nb\nc\n", "a\nB1\nc\n", "a\nB2\nc\n");
    assert_eq!(tally(&session), (0, 0, 0, 1));
    assert_eq!(session.apply_non_conflicting(ApplyScope::All), 0);
    assert_eq!(session.result_text(), "a\nb\nc\n");
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 0,
            conflicts: 1
        }
    );
    assert!(!session.is_touched(), "rien n'a été fait");
}

#[test]
fn case_06_contiguous_changes_abstain() {
    let (base, ours, theirs) = ("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_07_accept_both_in_click_order() {
    let (base, ours, theirs) = ("a\nb\n", "a\nX\nb\n", "a\nY\nb\n");
    let mut session = open(base, ours, theirs);
    assert_eq!(tally(&session), (0, 0, 0, 1));
    session.accept(0, Side::Left).unwrap();
    assert!(
        session.is_all_resolved(),
        "accepter un côté résout le conflit"
    );
    assert!(session.block(0).unwrap().accept_appends(Side::Right));
    session.accept(0, Side::Right).unwrap();
    assert_eq!(session.result_text(), "a\nX\nY\nb\n");
    check_invariants(&session);

    let mut session = open(base, ours, theirs);
    session.accept(0, Side::Right).unwrap();
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\nY\nX\nb\n");

    let mut session = open(base, ours, theirs);
    session.accept_both(0, Side::Right).unwrap();
    assert_eq!(session.result_text(), "a\nY\nX\nb\n");
    assert!(session.undo());
    assert_eq!(
        session.result_text(),
        base,
        "accept_both est une seule étape"
    );
}

#[test]
fn case_08_identical_insertion() {
    let (base, both) = ("a\nb\n", "a\nX\nb\n");
    assert_eq!(all(base, both, both, (0, 0, 1, 0)), "a\nX\nb\n");
}

#[test]
fn case_09_deletion_against_modification_abstains() {
    let (base, ours, theirs) = ("a\nb\nc\n", "a\nc\n", "a\nB\nc\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_10_identical_deletion() {
    let (base, both) = ("a\nb\nc\n", "a\nc\n");
    assert_eq!(all(base, both, both, (0, 0, 1, 0)), "a\nc\n");
}

#[test]
fn case_11_contiguous_insertion_abstains() {
    let (base, ours, theirs) = ("a\nb\nc\n", "a\nX\nb\nc\n", "a\nB\nc\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_12_whitespace_only_on_one_side_abstains() {
    let (base, ours, theirs) = ("  b\n", "    b\n", "  B\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_13_ignoring_keeps_the_base() {
    let mut session = open("a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\n");
    assert_eq!(tally(&session), (1, 0, 0, 0));
    session.ignore(0, Side::Left).unwrap();
    assert_eq!(session.counts(), MergeCounts::default());
    assert!(session.is_all_resolved());
    assert_eq!(session.result_text(), "a\nb\nc\n");
    // Un bloc ignoré n'est plus appliqué par « tous ».
    assert_eq!(session.apply_non_conflicting(ApplyScope::All), 0);
    assert_eq!(session.result_text(), "a\nb\nc\n");
}

#[test]
fn case_14_crlf_everywhere() {
    let base = "a\r\nb\r\nc\r\nd\r\ne\r\n";
    let ours = "a\r\nB\r\nc\r\nd\r\ne\r\n";
    let theirs = "a\r\nb\r\nc\r\nD\r\ne\r\n";
    assert_eq!(opening(base, ours, theirs), base);
    assert_eq!(
        all(base, ours, theirs, (1, 1, 0, 0)),
        "a\r\nB\r\nc\r\nD\r\ne\r\n"
    );
    let report = open(base, ours, theirs).line_endings();
    assert!(!report.differ, "pas de bandeau");
}

#[test]
fn case_15_their_conversion_alone_is_not_a_change() {
    let (base, ours, theirs) = ("a\nb\n", "a\nB\n", "a\r\nb\r\n");
    assert_eq!(all(base, ours, theirs, (1, 0, 0, 0)), "a\nB\n");
    let report = open(base, ours, theirs).line_endings();
    assert!(report.differ, "bandeau fins de ligne");
    assert_eq!(report.result, julie_diff::LineEnding::Lf);
}

#[test]
fn case_16_their_line_takes_our_line_endings() {
    let (base, theirs) = ("a\r\nb\r\n", "a\nb\nc\n");
    assert_eq!(all(base, base, theirs, (0, 1, 0, 0)), "a\r\nb\r\nc\r\n");
}

#[test]
fn case_17_mixed_file_is_not_normalised() {
    let (base, theirs) = ("a\r\nb\nc\r\n", "a\r\nb\nC\r\n");
    assert_eq!(opening(base, base, theirs), base);
    assert_eq!(all(base, base, theirs, (0, 1, 0, 0)), "a\r\nb\nC\r\n");
    assert!(!open(base, base, theirs).line_endings().differ);
}

#[test]
fn case_18_no_final_newline_kept() {
    let (base, ours) = ("a\nb", "a\nB");
    assert_eq!(opening(base, ours, base), base);
    assert_eq!(all(base, ours, base, (1, 0, 0, 0)), "a\nB");
}

#[test]
fn case_19_final_newline_added_by_them() {
    let (base, theirs) = ("a\nb", "a\nb\n");
    assert_eq!(all(base, base, theirs, (0, 1, 0, 0)), "a\nb\n");
}

#[test]
fn case_20_same_last_line_abstains() {
    let (base, ours, theirs) = ("a\nb", "a\nB", "a\nb\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_21_both_append_differently() {
    let (base, ours, theirs) = ("a\n", "a\nX\n", "a\nY\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_22_added_on_both_sides_is_one_block() {
    let mut session = open("", "x\ny\n", "x\nz\n");
    assert_eq!(tally(&session), (0, 0, 0, 1));
    assert_eq!(session.result_text(), "", "sans base, le centre part vide");
    let block = session.block(0).unwrap();
    assert_eq!(
        (block.ours.clone(), block.theirs.clone()),
        (0..2, 0..2),
        "un seul bloc, lignes communes comprises"
    );
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "x\ny\n");
}

#[test]
fn case_23_emptied_against_modified() {
    let (base, ours, theirs) = ("a\n", "", "A\n");
    assert_eq!(all(base, ours, theirs, (0, 0, 0, 1)), base);
}

#[test]
fn case_24_accept_then_undo() {
    let mut session = open("a\nb\nc\n", "a\nB1\nc\n", "a\nB2\nc\n");
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\nB1\nc\n");
    assert_eq!(session.counts(), MergeCounts::default());
    assert!(session.undo());
    assert_eq!(session.result_text(), "a\nb\nc\n");
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 0,
            conflicts: 1
        }
    );
    assert!(session.redo());
    assert_eq!(session.result_text(), "a\nB1\nc\n");
    assert_eq!(session.counts(), MergeCounts::default());
}

#[test]
fn case_25_bom_is_kept() {
    let base = "\u{feff}a\nb\nc\n";
    let ours = "\u{feff}a\nB\nc\n";
    assert_eq!(opening(base, ours, base), base);
    let result = all(base, ours, base, (1, 0, 0, 0));
    assert_eq!(result, "\u{feff}a\nB\nc\n");
    assert_eq!(&result.as_bytes()[..3], &[0xEF, 0xBB, 0xBF]);
}

#[test]
fn case_26_identical_and_conflict_in_one_file() {
    let base = "a\nb\nc\n1\n2\n3\na\nb\nc\nd\n";
    let ours = "a\nX\nc\n1\n2\n3\na\nB\nc\nd\n";
    let theirs = "a\nX\nc\n1\n2\n3\na\nb\nC\nd\n";
    assert_eq!(
        all(base, ours, theirs, (0, 0, 1, 1)),
        "a\nX\nc\n1\n2\n3\na\nb\nc\nd\n"
    );
}
