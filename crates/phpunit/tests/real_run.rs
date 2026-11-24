//! Essais avec un vrai PHP et le PHPUnit 9.6 de la machine, en lecture seule, sur
//! le petit projet de `tests/fixtures/phpunit`. Jamais sur un vrai projet.
//!
//! Sans PHP ou sans PHPUnit, chaque essai le dit et passe : cette crate se teste
//! aussi sur une machine qui n'a ni l'un ni l'autre (les essais d'analyse, eux,
//! tournent partout).

mod support;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use julie_core::process::Command;
use julie_core::{Bus, CancelToken, Project, TaskOutcome, Tasks};
use julie_phpunit::{
    execute, NodeKind, PhpUnit, PhpUnitEvent, PhpUnitSettings, Problem, RunOutcome, RunRequest,
    TestEvent, TestState,
};
use support::{collect_until_finished, fixture_project, real_phpunit};

struct Setup {
    bus: Bus,
    phpunit: PhpUnit,
    project: Project,
}

fn setup(script: PathBuf, tweak: impl FnOnce(&mut PhpUnitSettings)) -> Setup {
    let bus = Bus::new();
    let tasks = Tasks::new(bus.publisher());
    let project = Project::open(fixture_project()).unwrap();
    let mut settings = PhpUnitSettings {
        phpunit_path: Some(script),
        ..Default::default()
    };
    tweak(&mut settings);
    Setup {
        phpunit: PhpUnit::new(tasks, project.clone(), settings),
        bus,
        project,
    }
}

#[test]
fn a_full_run_fills_the_tree_and_matches_phpunits_own_summary() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script, |_| {});
    let handle = s.phpunit.run(RunRequest::all());
    let log = collect_until_finished(&s.bus, handle.id(), Duration::from_secs(60));
    handle.join();
    assert_eq!(log.outcome, TaskOutcome::Done);

    // Ordre des événements : RunStarted en premier, RunFinished en dernier.
    assert!(matches!(
        log.events.first().map(|r| &r.event),
        Some(PhpUnitEvent::RunStarted(_))
    ));
    assert!(matches!(
        log.events.last().map(|r| &r.event),
        Some(PhpUnitEvent::RunFinished(_))
    ));
    let PhpUnitEvent::RunStarted(start) = &log.events[0].event else {
        unreachable!()
    };
    assert!(start.command_line.contains("--teamcity"));
    assert!(start.command_line.contains("--configuration"));

    let summary = log.finished().unwrap();
    assert_eq!(summary.outcome, RunOutcome::Completed);
    assert!(!summary.is_green());
    assert!(summary.exit_code.is_some_and(|code| code != 0));

    // L'arbre que verrait l'interface.
    let tree = log.ui_tree(s.project.root());
    assert_eq!(tree.command(), Some(start.command_line.as_str()));
    assert!(tree.is_ended());
    let counts = tree.counts();
    assert_eq!(counts, &summary.counts);
    let reported = summary.reported.as_ref().expect("résumé de PHPUnit");
    assert_eq!(reported.tests, 13);
    assert_eq!(counts.started, 13);
    assert_eq!(counts.total, Some(13));
    assert_eq!(counts.passed, 6);
    assert_eq!(counts.errors as u32, reported.errors);
    assert_eq!(counts.failed as u32, reported.failures);
    assert_eq!(counts.skipped as u32, reported.skipped);
    assert_eq!(counts.incomplete as u32, reported.incomplete);
    assert_eq!(counts.risky as u32, reported.risky);
    assert_eq!(
        (
            reported.errors,
            reported.failures,
            reported.skipped,
            reported.incomplete,
            reported.risky
        ),
        (1, 3, 1, 1, 1)
    );

    // Les 3 jeux du fournisseur de données sont 3 nœuds nommés.
    let sets: Vec<_> = tree
        .iter()
        .filter(|node| node.kind == NodeKind::DataSet)
        .collect();
    assert_eq!(sets.len(), 3);
    // L'échec porte un lien vers le bon fichier et la bonne ligne.
    let failed = tree
        .iter()
        .find(|node| node.name == "testEqualsFailsWithExpectedAndActual")
        .unwrap();
    assert_eq!(failed.state, TestState::Failed);
    let location = failed
        .diagnostic
        .as_ref()
        .unwrap()
        .trace
        .iter()
        .find_map(|entry| entry.location.clone())
        .unwrap();
    assert!(location.path.ends_with("tests/Main/CalculatorTest.php"));
    assert_eq!(location.position.line, 24);
    // La sortie du test est dans son nœud.
    let writer = tree
        .iter()
        .find(|node| node.name == "testWritesOnOutput")
        .unwrap();
    assert_eq!(writer.console_text(), "texte écrit par le test\n");
}

#[test]
fn nodes_arrive_while_a_slow_test_is_still_running() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script, |_| {});
    let handle = s.phpunit.run(RunRequest::all());
    let log = collect_until_finished(&s.bus, handle.id(), Duration::from_secs(60));
    handle.join();

    let slow_started = log
        .first_arrival(
            |event| matches!(event, TestEvent::TestStarted { name, .. } if name == "testIsSlow"),
        )
        .expect("testIsSlow annoncé");
    let finished = log
        .events
        .iter()
        .find(|r| matches!(r.event, PhpUnitEvent::RunFinished(_)))
        .unwrap()
        .at;
    // Le test dort 1 s : son nœud est arrivé bien avant la fin de l'exécution.
    let gap = finished.duration_since(slow_started);
    assert!(
        gap >= Duration::from_millis(700),
        "écart de seulement {gap:?}"
    );
    // Et par lots, pas un événement de bus par ligne.
    let batches = log
        .events
        .iter()
        .filter(|r| matches!(r.event, PhpUnitEvent::Events(_)))
        .count();
    assert!(batches < 30, "{batches} lots");
}

#[test]
fn a_test_that_exits_ends_the_run_as_aborted() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script, |_| {});
    let handle = s.phpunit.run(RunRequest::test_suite("crash"));
    let log = collect_until_finished(&s.bus, handle.id(), Duration::from_secs(60));
    handle.join();
    assert_eq!(log.outcome, TaskOutcome::Done, "l'IDE ne plante pas");

    let summary = log.finished().unwrap();
    assert_eq!(summary.outcome, RunOutcome::Aborted { exit_code: Some(1) });
    assert!(!summary.is_green());
    assert_eq!(summary.counts.errors, 1);
    assert_eq!(summary.counts.passed, 1);
    let tree = log.ui_tree(s.project.root());
    let crashed = tree
        .iter()
        .find(|node| node.name == "testExitsTheProcess")
        .unwrap();
    assert_eq!(crashed.state, TestState::Error);
    assert_eq!(crashed.console_text(), "avant exit\n");
    assert!(tree.iter().all(|node| node.name != "testNeverReached"));
    assert!(tree.is_ended());
}

fn is_php_alive(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .run(&CancelToken::new())
        .unwrap();
    output.stdout_text().to_ascii_lowercase().contains("php")
}

#[test]
fn stopping_kills_php_within_two_seconds() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script, |_| {});
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid.txt");
    let mut request = RunRequest::test_suite("slow");
    request.env.push((
        "JULIE_PID_FILE".into(),
        pid_file.to_string_lossy().into_owned(),
    ));
    let handle = s.phpunit.run(request);
    let id = handle.id();

    // Attend que le test dorme réellement (le processus a écrit son numéro).
    let waited = Instant::now();
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                break pid;
            }
        }
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "PHP ne démarre pas"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(is_php_alive(pid));

    let stopped_at = Instant::now();
    s.phpunit.stop(id);
    let log = collect_until_finished(&s.bus, id, Duration::from_secs(10));
    let elapsed = stopped_at.elapsed();
    handle.join();
    assert!(elapsed < Duration::from_secs(2), "arrêt en {elapsed:?}");
    assert_eq!(log.outcome, TaskOutcome::Cancelled);
    assert!(!is_php_alive(pid), "le processus PHP {pid} existe encore");

    let summary = log.finished().expect("RunFinished malgré l'arrêt");
    assert_eq!(summary.outcome, RunOutcome::Stopped);
    let tree = log.ui_tree(s.project.root());
    let sleeping = tree
        .iter()
        .find(|node| node.name == "testSleepsThirtySeconds")
        .unwrap();
    assert_eq!(sleeping.state, TestState::Interrupted);
    assert_eq!(summary.counts.interrupted, 1);
}

#[test]
fn a_run_that_exceeds_its_time_limit_is_stopped_by_the_service() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script, |settings| settings.timeout_seconds = Some(2));
    let started = Instant::now();
    let handle = s.phpunit.run(RunRequest::test_suite("slow"));
    let log = collect_until_finished(&s.bus, handle.id(), Duration::from_secs(20));
    handle.join();
    assert!(started.elapsed() < Duration::from_secs(10));
    // Le délai n'est pas une annulation demandée par l'utilisatrice.
    assert_eq!(log.outcome, TaskOutcome::Done);
    assert_eq!(log.finished().unwrap().outcome, RunOutcome::TimedOut);
}

#[test]
fn a_filter_runs_only_what_it_matches() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script, |_| {});
    let handle = s.phpunit.run(RunRequest::filter("testAdditionPasses"));
    let log = collect_until_finished(&s.bus, handle.id(), Duration::from_secs(60));
    handle.join();
    let summary = log.finished().unwrap();
    assert_eq!(summary.outcome, RunOutcome::Completed);
    assert_eq!((summary.counts.started, summary.counts.passed), (1, 1));
    assert!(summary.is_green());
}

#[test]
fn detection_reads_the_versions() {
    let Some(script) = real_phpunit() else { return };
    let s = setup(script.clone(), |_| {});
    let handle = s.phpunit.detect();
    let log = collect_until_finished(&s.bus, handle.id(), Duration::from_secs(60));
    handle.join();
    let Some(PhpUnitEvent::Detected(detection)) = log.events.first().map(|r| r.event.clone())
    else {
        panic!("Detected attendu");
    };
    assert!(detection.is_ready(), "{:?}", detection.problems());
    let php = detection.php.as_ref().unwrap();
    assert!(php
        .version
        .as_deref()
        .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit())));
    let unit = detection.phpunit.as_ref().unwrap();
    assert_eq!(unit.path, script);
    assert_eq!(unit.version.as_ref().map(|v| v.major), Some(9));
    assert_eq!(
        detection.configuration.as_ref().unwrap(),
        &fixture_project().join("phpunit.xml")
    );
}

fn collect_events(
    project: &Project,
    settings: &PhpUnitSettings,
) -> (
    Result<julie_phpunit::RunSummary, julie_phpunit::PhpUnitError>,
    Vec<PhpUnitEvent>,
) {
    let events = Mutex::new(Vec::new());
    let result = execute(
        project,
        settings,
        &RunRequest::all(),
        &CancelToken::new(),
        &|event| events.lock().unwrap().push(event),
    );
    (result, events.into_inner().unwrap())
}

#[test]
fn problems_are_reported_before_anything_is_launched() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project::open(dir.path()).unwrap();

    // Ni PHPUnit ni configuration.
    let (result, events) = collect_events(&project, &PhpUnitSettings::default());
    assert!(events.is_empty());
    let error = result.unwrap_err();
    // Selon la machine PHP peut manquer aussi : le premier problème est rendu.
    assert!(matches!(
        error,
        julie_phpunit::PhpUnitError::Launch(Problem::PhpNotFound { .. })
            | julie_phpunit::PhpUnitError::Launch(Problem::PhpUnitNotFound { .. })
    ));

    // Un chemin de PHP invalide est nommé.
    let settings = PhpUnitSettings {
        php_path: Some(dir.path().join("pas-de-php.exe")),
        ..Default::default()
    };
    let (result, events) = collect_events(&project, &settings);
    assert!(events.is_empty());
    match result.unwrap_err() {
        julie_phpunit::PhpUnitError::Launch(Problem::PhpPathInvalid(path)) => {
            assert!(path.ends_with("pas-de-php.exe"));
        }
        other => panic!("{other:?}"),
    }

    // PHPUnit présent, mais ni phpunit.xml ni phpunit.xml.dist.
    let Some(script) = real_phpunit() else { return };
    let settings = PhpUnitSettings {
        phpunit_path: Some(script),
        ..Default::default()
    };
    let (result, events) = collect_events(&project, &settings);
    assert!(events.is_empty());
    assert!(matches!(
        result.unwrap_err(),
        julie_phpunit::PhpUnitError::Launch(Problem::NoConfiguration { .. })
    ));
}

#[test]
fn a_launch_problem_is_published_and_fails_the_task() {
    let dir = tempfile::tempdir().unwrap();
    let bus = Bus::new();
    let tasks = Tasks::new(bus.publisher());
    let project = Project::open(dir.path()).unwrap();
    let settings = PhpUnitSettings {
        php_path: Some(dir.path().join("pas-de-php.exe")),
        ..Default::default()
    };
    let phpunit = PhpUnit::new(tasks, project, settings);
    let handle = phpunit.run(RunRequest::all());
    let log = collect_until_finished(&bus, handle.id(), Duration::from_secs(20));
    handle.join();
    assert!(matches!(log.outcome, TaskOutcome::Failed(_)));
    assert_eq!(log.events.len(), 1);
    assert!(matches!(
        &log.events[0].event,
        PhpUnitEvent::LaunchFailed(Problem::PhpPathInvalid(_))
    ));
}
