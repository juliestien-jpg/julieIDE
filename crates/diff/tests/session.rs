//! Comportement de la session de fusion au-delà des 26 cas de la spécification :
//! frappe libre, annulation, états des côtés, navigation, correspondance des
//! volets, fins de ligne.

mod common;

use common::{check_invariants, open};
use julie_core::TextPosition;
use julie_diff::{
    AlignedSegment, ApplyScope, BlockKind, LineEnding, MergeCounts, SessionError, Side, SideState,
};

fn pos(line: u32, column: u32) -> TextPosition {
    TextPosition::new(line, column)
}

const CONFLICT: (&str, &str, &str) = ("a\nb\nc\n", "a\nB1\nc\n", "a\nB2\nc\n");

#[test]
fn typing_in_a_pending_block_does_not_resolve_it() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.replace_lines(1..2, &["tapé"]).unwrap();
    assert_eq!(session.result_text(), "a\ntapé\nc\n");
    assert_eq!(session.counts().conflicts, 1, "taper ne résout pas");
    assert_eq!(session.block(0).unwrap().result, 1..2);
    assert!(session.block(0).unwrap().can_accept(Side::Left));

    // Accepter remplace le texte tapé…
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\nB1\nc\n");
    // … et c'est annulable, étape par étape.
    assert!(session.undo());
    assert_eq!(session.result_text(), "a\ntapé\nc\n");
    assert_eq!(session.counts().conflicts, 1);
    assert!(session.undo());
    assert_eq!(session.result_text(), base);
    assert!(!session.undo());
    assert!(!session.is_touched());
    check_invariants(&session);
}

#[test]
fn a_block_grows_and_shrinks_with_the_text_typed_in_it() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session
        .replace_lines(1..2, &["un", "deux", "trois"])
        .unwrap();
    assert_eq!(session.block(0).unwrap().result, 1..4);
    session.replace_lines(2..3, &[]).unwrap();
    assert_eq!(session.block(0).unwrap().result, 1..3);
    assert_eq!(session.result_text(), "a\nun\ntrois\nc\n");
    session.accept(0, Side::Right).unwrap();
    assert_eq!(session.result_text(), "a\nB2\nc\n");
    check_invariants(&session);
}

#[test]
fn typing_outside_blocks_moves_them_without_touching_them() {
    let mut session = open("a\nb\nc\nd\n", "a\nb\nc\nD\n", "a\nb\nc\nd\n");
    assert_eq!(session.block(0).unwrap().result, 3..4);
    session.replace_lines(0..0, &["x", "y"]).unwrap();
    assert_eq!(session.block(0).unwrap().result, 5..6);
    session.replace_lines(2..4, &[]).unwrap();
    assert_eq!(session.block(0).unwrap().result, 3..4);
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "x\ny\nc\nD\n");
    check_invariants(&session);
}

#[test]
fn inserting_right_at_a_block_edge_stays_outside_the_block() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.replace_lines(1..1, &["avant"]).unwrap();
    assert_eq!(session.block(0).unwrap().result, 2..3);
    session.replace_lines(3..3, &["après"]).unwrap();
    assert_eq!(session.block(0).unwrap().result, 2..3);
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\navant\nB1\naprès\nc\n");
}

#[test]
fn a_replacement_across_a_block_edge_leaves_the_typed_text_outside() {
    // Lignes : 0 a, 1 b (bloc), 2 c. On remplace b et c d'un coup.
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.replace_lines(1..3, &["neuf"]).unwrap();
    assert_eq!(session.result_text(), "a\nneuf\n");
    let block = session.block(0).unwrap();
    assert_eq!(block.result, 1..2, "le bloc garde le texte tapé à sa place");
    check_invariants(&session);
    // On remplace a et b : le bloc commence dans le remplacement, il se vide.
    let mut session = open(base, ours, theirs);
    session.replace_lines(0..2, &["neuf"]).unwrap();
    assert_eq!(session.block(0).unwrap().result, 1..1);
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "neuf\nB1\nc\n");
    check_invariants(&session);
}

#[test]
fn retyping_the_same_text_changes_nothing() {
    let mut session = open("a\r\nb\r\n", "a\r\nb\r\n", "a\r\nb\r\n");
    session.replace_lines(0..2, &["a", "b"]).unwrap();
    assert!(!session.is_touched());
    assert_eq!(session.result_text(), "a\r\nb\r\n");
}

#[test]
fn character_level_edits() {
    let mut session = open("abc\ndef\n", "abc\ndef\n", "abc\ndef\n");
    session.edit(pos(0, 1), pos(0, 2), "XY").unwrap();
    assert_eq!(session.result_text(), "aXYc\ndef\n");
    // Saut de ligne tapé au milieu d'une ligne.
    session.edit(pos(1, 1), pos(1, 1), "\n").unwrap();
    assert_eq!(session.result_text(), "aXYc\nd\nef\n");
    // Retour arrière sur un saut de ligne : les deux lignes se joignent.
    session.edit(pos(1, 1), pos(2, 0), "").unwrap();
    assert_eq!(session.result_text(), "aXYc\ndef\n");
    // Sélection sur plusieurs lignes remplacée par plusieurs lignes (CRLF collé).
    session.edit(pos(0, 2), pos(1, 2), "1\r\n2\r\n3").unwrap();
    assert_eq!(session.result_text(), "aX1\n2\n3f\n");
    // À la fin du texte.
    let end = pos(session.result_line_count() as u32, 0);
    session.edit(end, end, "fin").unwrap();
    assert_eq!(session.result_text(), "aX1\n2\n3f\nfin\n");
    // Bornes inversées acceptées ; colonnes en caractères, pas en octets.
    let mut session = open("éà€\n", "éà€\n", "éà€\n");
    session.edit(pos(0, 3), pos(0, 1), "").unwrap();
    assert_eq!(session.result_text(), "é\n");
    assert_eq!(
        session.edit(pos(0, 9), pos(0, 9), "x"),
        Err(SessionError::OutOfRange)
    );
    assert_eq!(
        session.edit(pos(5, 0), pos(5, 0), "x"),
        Err(SessionError::OutOfRange)
    );
    check_invariants(&session);
}

#[test]
fn editing_a_line_keeps_its_lone_carriage_returns() {
    // « a\r » suivi de CRLF : le premier \r fait partie du contenu de la ligne.
    let text = "a\r\r\nb\r\n";
    let mut session = open(text, text, text);
    session.edit(pos(0, 0), pos(0, 0), "x").unwrap();
    assert_eq!(session.result_text(), "xa\r\r\nb\r\n");
    // Texte collé en CRLF, terminé par un \r isolé : les CRLF sont des sauts de
    // ligne, le \r final est du contenu.
    session.edit(pos(1, 1), pos(1, 1), "1\r\n2\r").unwrap();
    assert_eq!(session.result_text(), "xa\r\r\nb1\r\n2\r\r\n");
}

#[test]
fn deleting_everything_and_typing_again() {
    let mut session = open("a\nb\n", "a\nb\n", "a\nb\n");
    session.edit(pos(0, 0), pos(2, 0), "").unwrap();
    assert_eq!(session.result_line_count(), 0);
    assert_eq!(session.result_text(), "");
    session.edit(pos(0, 0), pos(0, 0), "x\ny").unwrap();
    assert_eq!(session.result_text(), "x\ny\n");
    while session.undo() {}
    assert_eq!(session.result_text(), "a\nb\n");
}

#[test]
fn a_typed_last_line_follows_our_final_newline() {
    // Notre version finit par un saut de ligne : la ligne tapée en dernier aussi.
    let mut session = open("a\n", "a\n", "a\n");
    session.replace_lines(1..1, &["tapé"]).unwrap();
    assert_eq!(session.result_text(), "a\ntapé\n");
    // Notre version n'en a pas : la ligne tapée non plus, et l'ancienne
    // dernière ligne reçoit la fin de ligne qui lui manquait.
    let mut session = open("a\nb", "a\nb", "a\nb");
    session.replace_lines(2..2, &["tapé"]).unwrap();
    assert_eq!(session.result_text(), "a\nb\ntapé");
}

#[test]
fn ignoring_one_side_of_a_conflict_is_not_enough() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.ignore(0, Side::Left).unwrap();
    assert_eq!(session.counts().conflicts, 1);
    let block = session.block(0).unwrap();
    assert_eq!(
        (block.left, block.right),
        (SideState::Ignored, SideState::Pending)
    );
    assert!(!block.resolved);
    assert!(!block.can_ignore(Side::Left) && block.can_ignore(Side::Right));

    session.ignore(0, Side::Right).unwrap();
    assert!(session.is_all_resolved());
    assert_eq!(
        session.result_text(),
        base,
        "les deux ignorés : le centre ne bouge pas"
    );

    // Annuler rend le compteur.
    session.undo();
    assert_eq!(session.counts().conflicts, 1);
}

#[test]
fn accepting_after_ignoring() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.ignore(0, Side::Left).unwrap();
    session.accept(0, Side::Right).unwrap();
    assert!(session.is_all_resolved());
    assert_eq!(session.result_text(), "a\nB2\nc\n");
    // On peut se raviser sur le côté ignoré : il s'ajoute à la suite.
    assert!(session.block(0).unwrap().accept_appends(Side::Left));
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\nB2\nB1\nc\n");

    // Bloc simple : ignoré puis accepté.
    let mut session = open("a\nb\n", "a\nB\n", "a\nb\n");
    session.ignore(0, Side::Left).unwrap();
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\nB\n");
}

#[test]
fn appending_goes_after_what_was_typed_in_the_block() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.accept(0, Side::Left).unwrap();
    session.replace_lines(2..2, &["note"]).unwrap();
    // Une insertion pile à la fin d'un bloc tombe après lui, hors du bloc.
    assert_eq!(session.block(0).unwrap().result, 1..2);
    session
        .replace_lines(1..2, &["B1", "dans le bloc"])
        .unwrap();
    assert_eq!(session.block(0).unwrap().result, 1..3);
    session.accept(0, Side::Right).unwrap();
    assert_eq!(session.result_text(), "a\nB1\ndans le bloc\nB2\nnote\nc\n");
    check_invariants(&session);
}

#[test]
fn refused_actions() {
    let mut session = open("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nd\nE\n");
    assert_eq!(
        session.accept(9, Side::Left),
        Err(SessionError::UnknownBlock(9))
    );
    assert_eq!(
        session.ignore(9, Side::Left),
        Err(SessionError::UnknownBlock(9))
    );
    assert_eq!(
        session.accept(0, Side::Right),
        Err(SessionError::SideNotApplicable)
    );
    assert_eq!(
        session.ignore(1, Side::Left),
        Err(SessionError::SideNotApplicable)
    );
    assert_eq!(
        session.accept_both(0, Side::Left),
        Err(SessionError::NotAConflict)
    );
    session.accept(0, Side::Left).unwrap();
    assert_eq!(
        session.accept(0, Side::Left),
        Err(SessionError::AlreadyAccepted)
    );
    assert_eq!(
        session.ignore(0, Side::Left),
        Err(SessionError::AlreadyAccepted)
    );
    session.ignore(1, Side::Right).unwrap();
    assert_eq!(
        session.ignore(1, Side::Right),
        Err(SessionError::AlreadyIgnored)
    );
    assert_eq!(
        session.replace_lines(4..9, &[]),
        Err(SessionError::OutOfRange)
    );
    // Un refus ne laisse pas de trace dans l'historique.
    assert!(session.undo());
    assert!(session.undo());
    assert!(!session.undo());
    assert_eq!(session.result_text(), "a\nb\nc\nd\ne\n");
}

#[test]
fn identical_blocks_have_a_single_pair_of_controls() {
    let mut session = open("a\nb\n", "a\nX\n", "a\nX\n");
    let block = session.block(0).unwrap();
    assert_eq!(block.kind, BlockKind::Identical);
    session.accept(0, Side::Right).unwrap();
    let block = session.block(0).unwrap();
    assert_eq!(
        (block.left, block.right),
        (SideState::Accepted, SideState::Accepted)
    );
    assert_eq!(
        session.accept(0, Side::Left),
        Err(SessionError::AlreadyAccepted)
    );
    assert_eq!(session.result_text(), "a\nX\n", "appliqué une seule fois");
}

/// Gauche, droite, identique et conflit dans un même fichier.
fn four_kinds() -> (&'static str, &'static str, &'static str) {
    (
        "1\na\n2\n3\nb\n4\n5\nc\n6\n7\nd\n8\n",
        "1\nA\n2\n3\nb\n4\n5\nC\n6\n7\nD1\n8\n",
        "1\na\n2\n3\nB\n4\n5\nC\n6\n7\nD2\n8\n",
    )
}

#[test]
fn apply_scopes() {
    let (base, ours, theirs) = four_kinds();
    let kinds: Vec<BlockKind> = open(base, ours, theirs)
        .blocks()
        .iter()
        .map(|b| b.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            BlockKind::Left,
            BlockKind::Right,
            BlockKind::Identical,
            BlockKind::Conflict
        ]
    );

    let mut session = open(base, ours, theirs);
    assert_eq!(session.apply_non_conflicting(ApplyScope::Left), 2);
    assert_eq!(
        session.result_text(),
        "1\nA\n2\n3\nb\n4\n5\nC\n6\n7\nd\n8\n"
    );
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 1,
            conflicts: 1
        }
    );
    assert_eq!(
        session.apply_non_conflicting(ApplyScope::Left),
        0,
        "rien de plus à faire"
    );

    let mut session = open(base, ours, theirs);
    assert_eq!(session.apply_non_conflicting(ApplyScope::Right), 2);
    assert_eq!(
        session.result_text(),
        "1\na\n2\n3\nB\n4\n5\nC\n6\n7\nd\n8\n"
    );
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 1,
            conflicts: 1
        }
    );

    let mut session = open(base, ours, theirs);
    assert_eq!(session.apply_non_conflicting(ApplyScope::All), 3);
    assert_eq!(
        session.result_text(),
        "1\nA\n2\n3\nB\n4\n5\nC\n6\n7\nd\n8\n"
    );
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 0,
            conflicts: 1
        }
    );
    assert_eq!(
        session.block(3).unwrap().left,
        SideState::Pending,
        "conflit intact"
    );
    check_invariants(&session);

    // Une seule étape à annuler, compteur compris.
    assert!(session.undo());
    assert_eq!(session.result_text(), base);
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 3,
            conflicts: 1
        }
    );
    assert!(!session.can_undo());
    assert!(session.redo());
    assert_eq!(
        session.result_text(),
        "1\nA\n2\n3\nB\n4\n5\nC\n6\n7\nd\n8\n"
    );
    assert_eq!(
        session.counts(),
        MergeCounts {
            changes: 0,
            conflicts: 1
        }
    );
}

#[test]
fn apply_skips_blocks_already_handled_and_keeps_typed_text_elsewhere() {
    let (base, ours, theirs) = four_kinds();
    let mut session = open(base, ours, theirs);
    session.ignore(0, Side::Left).unwrap();
    session.replace_lines(0..1, &["un"]).unwrap();
    assert_eq!(session.apply_non_conflicting(ApplyScope::All), 2);
    assert_eq!(
        session.result_text(),
        "un\na\n2\n3\nB\n4\n5\nC\n6\n7\nd\n8\n"
    );
}

#[test]
fn a_new_action_clears_redo() {
    let (base, ours, theirs) = CONFLICT;
    let mut session = open(base, ours, theirs);
    session.accept(0, Side::Left).unwrap();
    session.undo();
    assert!(session.can_redo());
    session.accept(0, Side::Right).unwrap();
    assert!(!session.can_redo());
    assert!(!session.redo());
    assert_eq!(session.result_text(), "a\nB2\nc\n");
}

#[test]
fn grouping_steps() {
    let mut session = open("a\n", "a\n", "a\n");
    session.edit(pos(0, 1), pos(0, 1), "b").unwrap();
    session.edit(pos(0, 2), pos(0, 2), "c").unwrap();
    assert!(session.merge_last_two_steps());
    session.edit(pos(0, 3), pos(0, 3), "\nd").unwrap();
    assert!(session.merge_last_two_steps());
    assert!(!session.merge_last_two_steps(), "il ne reste qu'une étape");
    assert_eq!(session.result_text(), "abc\nd\n");
    assert!(session.undo());
    assert_eq!(session.result_text(), "a\n");
    assert!(session.redo());
    assert_eq!(session.result_text(), "abc\nd\n");
}

#[test]
fn navigation_between_pending_blocks() {
    let (base, ours, theirs) = four_kinds();
    let mut session = open(base, ours, theirs);
    assert_eq!(session.next_pending(None), Some(0));
    assert_eq!(session.next_pending(Some(0)), Some(1));
    assert_eq!(session.next_pending(Some(3)), None);
    assert_eq!(session.previous_pending(None), Some(3));
    assert_eq!(session.previous_pending(Some(1)), Some(0));
    assert_eq!(session.previous_pending(Some(0)), None);
    session.apply_non_conflicting(ApplyScope::All);
    assert_eq!(session.next_pending(None), Some(3), "seul le conflit reste");
    assert_eq!(session.previous_pending(Some(3)), None);

    assert_eq!(session.block_at_result_line(0), None);
    assert_eq!(session.block_at_result_line(1), Some(0));
    assert_eq!(session.block_at_result_line(10), Some(3));
    assert_eq!(session.block_at_result_line(99), None);
}

#[test]
fn alignment_of_the_three_panes() {
    // Notre côté insère deux lignes ; le leur supprime la dernière.
    let mut session = open("a\nb\nc\n", "a\nX\nY\nb\nc\n", "a\nb\n");
    let segment = |left, result, right, block| AlignedSegment {
        left,
        result,
        right,
        block,
    };
    assert_eq!(
        session.alignment(),
        vec![
            segment(0..1, 0..1, 0..1, None),
            segment(1..3, 1..1, 1..1, Some(0)),
            segment(3..4, 1..2, 1..2, None),
            segment(4..5, 2..3, 2..2, Some(1)),
        ]
    );
    // Un bloc vide au centre se trouve par la ligne où il s'insère.
    assert_eq!(session.block_at_result_line(1), Some(0));

    session.apply_non_conflicting(ApplyScope::All);
    assert_eq!(
        session.alignment(),
        vec![
            segment(0..1, 0..1, 0..1, None),
            segment(1..3, 1..3, 1..1, Some(0)),
            segment(3..4, 3..4, 1..2, None),
            segment(4..5, 4..4, 2..2, Some(1)),
        ]
    );
    assert_eq!(session.result_text(), "a\nX\nY\nb\n");
    check_invariants(&session);
}

#[test]
fn block_shapes_for_colouring() {
    use julie_diff::HunkKind;
    let session = open(
        "a\nb\nc\nd\ne\nf\n",
        "a\nX\nb\nc\nd\nE\nf\n",
        "a\nb\nc\ne\nf\n",
    );
    let blocks = session.blocks();
    assert_eq!(blocks[0].shape(Side::Left), HunkKind::Added);
    // d supprimé en face, e modifié chez nous : lignes voisines, donc conflit.
    assert_eq!(blocks[1].kind, BlockKind::Conflict);
    assert_eq!(blocks[1].shape(Side::Left), HunkKind::Modified);
    let session = open("a\nb\nc\n", "a\nc\n", "a\nb\nc\n");
    assert_eq!(session.blocks()[0].shape(Side::Left), HunkKind::Removed);
}

#[test]
fn typing_next_to_an_empty_block_does_not_fill_it() {
    let mut session = open("a\nb\n", "a\nX\nb\n", "a\nb\n");
    assert_eq!(session.block(0).unwrap().result, 1..1);
    session.edit(pos(1, 0), pos(1, 1), "bb").unwrap();
    assert_eq!(session.block(0).unwrap().result, 1..1);
    session.replace_lines(1..1, &["tapé"]).unwrap();
    assert_eq!(
        session.block(0).unwrap().result,
        2..2,
        "l'insertion passe avant le bloc vide"
    );
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\ntapé\nX\nbb\n");
}

#[test]
fn accepted_lines_never_get_glued_together() {
    // La dernière ligne de notre version n'a pas de saut de ligne ; acceptée
    // puis suivie du texte d'en face, elle doit en recevoir un.
    let mut session = open("a\nb", "a\nB", "a\nb\nc\n");
    assert_eq!(session.blocks()[0].kind, BlockKind::Conflict);
    session.accept(0, Side::Left).unwrap();
    assert_eq!(session.result_text(), "a\nB");
    session.accept(0, Side::Right).unwrap();
    assert_eq!(session.result_text(), "a\nB\nb\nc\n");
    // Dans l'autre ordre, notre ligne finit le fichier : pas de saut final.
    let mut session = open("a\nb", "a\nB", "a\nb\nc\n");
    session.accept_both(0, Side::Right).unwrap();
    assert_eq!(session.result_text(), "a\nb\nc\nB");
}

#[test]
fn line_endings_follow_our_version_line_by_line() {
    // Notre version : CRLF dominant, une ligne en LF. La leur : tout en LF.
    let base = "a\r\nb\nc\r\nd\r\n";
    let ours = "a\r\nb\nc\r\nD\n";
    let theirs = "A\nb\nc\nd\n";
    let mut session = open(base, ours, theirs);
    let report = session.line_endings();
    assert!(report.differ);
    assert_eq!(report.result, LineEnding::CrLf);
    session.apply_non_conflicting(ApplyScope::All);
    // A vient d'en face : style dominant (CRLF). b garde son LF. D est notre
    // ligne : elle garde son LF à elle.
    assert_eq!(session.result_text(), "A\r\nb\nc\r\nD\n");
}

#[test]
fn line_ending_report_ignores_versions_without_any_line_ending() {
    let session = open("", "a\r\nb\r\n", "a");
    let report = session.line_endings();
    assert!(!report.differ);
    assert_eq!(report.result, LineEnding::CrLf);
    // Notre version sans fin de ligne : on prend le style d'en face.
    let session = open("", "a", "a\r\nb\r\n");
    assert_eq!(session.line_endings().result, LineEnding::CrLf);
}

#[test]
fn sources_are_readable_from_the_session() {
    let session = open("a\n", "\u{feff}a\nb\n", "a\n");
    assert_eq!(session.ours().len(), 2);
    assert_eq!(session.ours().content(1), "b");
    assert_eq!(session.base().len(), 1);
    assert_eq!(session.theirs().content(0), "a");
    assert!(session.result_has_bom());
    assert_eq!(session.result_lines().collect::<Vec<_>>(), vec!["a"]);
    assert_eq!(session.block_count(), 1);
    assert_eq!(session.chunks().len(), 2);
}
