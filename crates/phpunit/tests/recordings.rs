//! Analyse des sorties enregistrées : un test par format.
//!
//! - `phpunit-9.6.34-*` : enregistrées avec un vrai PHPUnit 9.6.34.
//! - `unverified-*` : écrites de mémoire pour PHPUnit 10, 11 et 12, que cette
//!   machine n'a pas. Elles vérifient la tolérance de l'analyseur, pas la
//!   fidélité au format réel.

mod support;

use julie_core::process::Stream;
use julie_phpunit::{
    AbortReason, DiagnosticCause, NodeKind, ResultTree, TestEvent, TestNode, TestState,
};
use support::{recording, replay, tree_of};

fn find<'a>(tree: &'a ResultTree, name: &str) -> &'a TestNode {
    tree.iter()
        .find(|node| node.name == name)
        .unwrap_or_else(|| panic!("nœud {name} introuvable"))
}

/// La source des essais enregistrés : les chemins sont ceux de cette machine.
fn nine_main() -> ResultTree {
    // Les marqueurs risqué / erreur viennent du message ; incomplet / ignoré du
    // code source (voir `julie_phpunit::refine`), appliqué ici comme le service.
    let mut events = replay(&recording("phpunit-9.6.34-main.stdout"));
    let root = support::fixture_project();
    let mut sources = julie_phpunit::SourceLines::default();
    for event in &mut events {
        julie_phpunit::refine(event, &root, &mut sources);
    }
    events.push(TestEvent::Ended);
    tree_of(&events, &root.to_string_lossy())
}

#[test]
fn phpunit_9_6_full_run_has_the_expected_tree() {
    let tree = nine_main();
    let main = tree.children(tree.root())[0];
    assert_eq!(tree.node(main).name, "main");
    assert_eq!(tree.node(main).kind, NodeKind::Suite);
    let class = tree.children(main)[0];
    assert_eq!(tree.node(class).kind, NodeKind::Class);
    assert_eq!(tree.node(class).label, "CalculatorTest");
    let methods: Vec<&str> = tree
        .children(class)
        .iter()
        .map(|id| tree.node(*id).label.as_str())
        .collect();
    assert_eq!(
        methods,
        [
            "testAdditionPasses",
            "testSecondAssertionPasses",
            "testEqualsFailsWithExpectedAndActual",
            "testMultilineEqualsFails",
            "testThrowsAnException",
            "testIsSkipped",
            "testIsIncomplete",
            "testIsRisky",
            "testAdditionWithDataSet",
            "testWritesOnOutput",
            "testIsSlow",
        ]
    );
}

#[test]
fn phpunit_9_6_data_set_names_survive_escaping() {
    let tree = nine_main();
    let group = tree
        .iter()
        .find(|node| node.name == "testAdditionWithDataSet")
        .unwrap();
    assert_eq!(group.kind, NodeKind::Method);
    assert!(!group.is_test);
    let labels: Vec<&str> = group
        .children
        .iter()
        .map(|id| tree.node(*id).label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "zéro plus zéro",
            "l'apostrophe | la barre [et] les crochets",
            "échoue"
        ]
    );
    assert!(group
        .children
        .iter()
        .all(|id| tree.node(*id).kind == NodeKind::DataSet));
    assert_eq!(tree.node(group.children[2]).state, TestState::Failed);
    assert_eq!(group.state, TestState::Failed);
}

#[test]
fn phpunit_9_6_counters_equal_phpunits_own_summary() {
    let tree = nine_main();
    let counts = tree.counts();
    let reported = tree.reported_summary().expect("résumé de PHPUnit");
    assert_eq!(reported.tests, 13);
    assert_eq!(counts.total, Some(13));
    assert_eq!(counts.started, 13);
    assert_eq!(counts.finished(), 13);
    assert_eq!(counts.running, 0);
    assert_eq!(counts.passed, 6);
    assert_eq!(counts.errors as u32, reported.errors);
    assert_eq!(counts.failed as u32, reported.failures);
    assert_eq!(counts.skipped as u32, reported.skipped);
    assert_eq!(counts.incomplete as u32, reported.incomplete);
    assert_eq!(counts.risky as u32, reported.risky);
    assert_eq!(tree.node(tree.root()).state, TestState::Error);
}

#[test]
fn phpunit_9_6_failures_carry_message_comparison_and_link() {
    let tree = nine_main();
    let equals = find(&tree, "testEqualsFailsWithExpectedAndActual");
    assert_eq!(equals.state, TestState::Failed);
    let diagnostic = equals.diagnostic.as_ref().unwrap();
    assert_eq!(
        diagnostic.message,
        "Failed asserting that two strings are equal."
    );
    let comparison = diagnostic.comparison.as_ref().unwrap();
    assert_eq!(comparison.expected_text(), "attendu");
    assert_eq!(comparison.actual_text(), "obtenu");
    let location = diagnostic
        .trace
        .iter()
        .find_map(|entry| entry.location.as_ref())
        .expect("un lien fichier:ligne");
    assert!(location.path.ends_with("tests/Main/CalculatorTest.php"));
    assert_eq!(location.position.line, 24); // ligne 25 de PHP
    assert!(location.path.is_file(), "{}", location.path.display());

    let multiline = find(&tree, "testMultilineEqualsFails");
    let comparison = multiline
        .diagnostic
        .as_ref()
        .unwrap()
        .comparison
        .as_ref()
        .unwrap();
    assert_eq!(comparison.expected_text(), "ligne 1\nligne 2\nligne 3");
    assert_eq!(comparison.actual_text(), "ligne 1\nligne deux\nligne 3");

    let error = find(&tree, "testThrowsAnException");
    assert_eq!(error.state, TestState::Error);
    assert_eq!(
        error.diagnostic.as_ref().unwrap().message,
        "RuntimeException : boum | avec 'apostrophes' et [crochets]"
    );
}

#[test]
fn phpunit_9_6_other_states() {
    let tree = nine_main();
    assert_eq!(find(&tree, "testIsSkipped").state, TestState::Skipped);
    assert_eq!(find(&tree, "testIsIncomplete").state, TestState::Incomplete);
    assert_eq!(find(&tree, "testIsRisky").state, TestState::Risky);
    assert_eq!(find(&tree, "testAdditionPasses").state, TestState::Passed);
    let skipped = find(&tree, "testIsSkipped").diagnostic.as_ref().unwrap();
    assert_eq!(skipped.message, "pas sur cette machine");
}

#[test]
fn phpunit_9_6_unrecognised_output_is_kept_on_the_right_node() {
    let tree = nine_main();
    let writer = find(&tree, "testWritesOnOutput");
    assert_eq!(writer.console_text(), "texte écrit par le test\n");
    let root = tree.node(tree.root()).console_text();
    assert!(root.starts_with("PHPUnit 9.6.34 by Sebastian Bergmann"));
    assert!(root.contains("texte écrit par le test\n"));
    assert!(root.contains("Time: 00:01.189, Memory: 6.00 MB"));
    assert!(root.contains("ERRORS!\nTests: 13, Assertions: 10, Errors: 1, Failures: 3, Skipped: 1, Incomplete: 1, Risky: 1."));
    // Aucune ligne `##teamcity` n'a fui dans la console.
    assert!(!root.contains("##teamcity"));
    assert_eq!(
        tree.phpunit_version().map(|v| v.text.as_str()),
        Some("9.6.34")
    );
}

#[test]
fn phpunit_9_6_durations_come_from_the_messages() {
    let tree = nine_main();
    assert_eq!(
        find(&tree, "testIsSlow").duration,
        Some(std::time::Duration::from_millis(1011))
    );
    let main = tree.children(tree.root())[0];
    assert!(tree.node(main).duration.unwrap() >= std::time::Duration::from_millis(1011));
}

#[test]
fn phpunit_9_6_crash_in_the_middle_of_the_suite() {
    let events = replay(&recording("phpunit-9.6.34-crash.stdout"));
    let mut tree = tree_of(&events, "C:/p");
    // Le flux s'arrête au milieu d'un test : rien n'est fermé.
    assert!(tree.has_open_work());
    assert_eq!(
        tree.current_test().map(|id| tree.node(id).name.as_str()),
        Some("testExitsTheProcess")
    );
    tree.apply(&TestEvent::Aborted(AbortReason::ProcessExited {
        exit_code: Some(1),
    }));
    tree.apply(&TestEvent::Ended);

    let crashed = find(&tree, "testExitsTheProcess");
    assert_eq!(crashed.state, TestState::Error);
    assert_eq!(
        crashed.diagnostic.as_ref().unwrap().cause,
        DiagnosticCause::ProcessExited { exit_code: Some(1) }
    );
    assert_eq!(crashed.console_text(), "avant exit\n");
    assert_eq!(find(&tree, "testBeforeTheCrash").state, TestState::Passed);
    // Le test jamais commencé n'est pas inventé, mais PHPUnit l'avait annoncé.
    assert!(tree.iter().all(|node| node.name != "testNeverReached"));
    assert_eq!(tree.counts().total, Some(3));
    assert_eq!(tree.counts().started, 2);
    assert_eq!(tree.counts().passed, 1);
    assert_eq!(tree.counts().errors, 1);
    assert_eq!(tree.node(tree.root()).state, TestState::Error);
    assert!(tree
        .iter()
        .filter(|n| n.kind != NodeKind::Root)
        .all(|n| n.is_test || n.finished));
}

#[test]
fn phpunit_10_style_output_without_framing_blank_lines() {
    let events = replay(&recording("unverified-phpunit-10-from-memory.stdout"));
    let mut events = events;
    events.push(TestEvent::Ended);
    let tree = tree_of(&events, r"C:\proj");

    assert_eq!(tree.phpunit_version().map(|v| v.major), Some(10));
    let counts = tree.counts();
    assert_eq!((counts.started, counts.passed, counts.failed), (4, 2, 1));
    assert_eq!(counts.ignored(), 1);

    // La suite d'un fournisseur de données se nomme `Classe::méthode`.
    let group = find(&tree, r"App\Tests\ThingTest::testProvided");
    assert_eq!(group.kind, NodeKind::Method);
    assert_eq!(group.label, "testProvided");
    let labels: Vec<&str> = group
        .children
        .iter()
        .map(|id| tree.node(*id).label.as_str())
        .collect();
    assert_eq!(labels, ["#0", "deux"]);

    let failed = find(&tree, "testFails");
    let comparison = failed
        .diagnostic
        .as_ref()
        .unwrap()
        .comparison
        .as_ref()
        .unwrap();
    assert_eq!(
        (
            comparison.expected_text().as_str(),
            comparison.actual_text().as_str()
        ),
        ("a", "b")
    );

    let reported = tree.reported_summary().unwrap();
    assert_eq!(
        (reported.tests, reported.failures, reported.skipped),
        (4, 1, 1)
    );
    // Pas de ligne vide parasite : celles de la sortie sont celles de PHPUnit.
    let root = tree.node(tree.root()).console_text();
    assert!(root.contains("Runtime:       PHP 8.3.1\n"));
}

#[test]
fn phpunit_11_12_style_names_prefixed_by_the_class_and_fractional_durations() {
    let mut events = replay(&recording("unverified-phpunit-11-12-from-memory.stdout"));
    events.push(TestEvent::Ended);
    let tree = tree_of(&events, r"C:\proj");

    assert_eq!(tree.phpunit_version().map(|v| v.major), Some(12));
    let works = find(&tree, r"App\Tests\OtherTest::testWorks");
    assert_eq!(works.label, "testWorks");
    assert_eq!(works.state, TestState::Passed);
    assert_eq!(works.duration, Some(std::time::Duration::from_micros(1250)));
    assert_eq!(
        find(&tree, r"App\Tests\OtherTest::testRisky").state,
        TestState::Risky
    );

    let boom = find(&tree, r"App\Tests\OtherTest::testBoom");
    assert_eq!(boom.state, TestState::Error);
    let trace = &boom.diagnostic.as_ref().unwrap().trace;
    assert_eq!(trace.len(), 2);
    assert_eq!(trace[0].location.as_ref().unwrap().position.line, 8);
    assert_eq!(trace[1].location.as_ref().unwrap().position.line, 21);

    let counts = tree.counts();
    let reported = tree.reported_summary().unwrap();
    assert_eq!(counts.errors as u32, reported.errors);
    assert_eq!(counts.risky as u32, reported.risky);
    assert_eq!(reported.deprecations, 1);
}

#[test]
fn every_line_of_every_recording_is_either_a_message_or_kept_output() {
    // Rien ne se perd : chaque ligne non vide qui n'est pas un message TeamCity
    // se retrouve dans la console de la racine, dans l'ordre.
    for name in [
        "phpunit-9.6.34-main.stdout",
        "phpunit-9.6.34-crash.stdout",
        "unverified-phpunit-10-from-memory.stdout",
        "unverified-phpunit-11-12-from-memory.stdout",
    ] {
        let text = recording(name);
        let expected: Vec<&str> = text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with("##teamcity["))
            .collect();
        let events = replay(&text);
        let kept: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                TestEvent::Output { stream, text }
                    if *stream == Stream::Stdout && !text.is_empty() =>
                {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(kept, expected, "{name}");
    }
}
