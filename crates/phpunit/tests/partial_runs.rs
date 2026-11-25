//! Lancer une partie de la suite, avec un vrai PHP et le PHPUnit 9.6 de la machine,
//! sur le petit projet de `tests/fixtures/phpunit` (jamais sur un vrai projet).
//! Sans PHP ou sans PHPUnit, chaque essai le dit et passe.

mod support;

use std::path::PathBuf;
use std::sync::Mutex;

use julie_core::{CancelToken, Project};
use julie_phpunit::{
    execute, preview_command, DataSetKey, NodeKind, PhpUnitEvent, PhpUnitSettings, ResultTree,
    RunRequest, RunSummary, TestState,
};
use support::{fixture_project, real_phpunit};

const CLASS: &str = r"Fixture\Tests\CalculatorTest";

struct Done {
    summary: RunSummary,
    tree: ResultTree,
    command_line: String,
}

fn settings(script: PathBuf) -> PhpUnitSettings {
    PhpUnitSettings {
        phpunit_path: Some(script),
        ..Default::default()
    }
}

fn run(settings: &PhpUnitSettings, request: &RunRequest) -> Done {
    let project = Project::open(fixture_project()).unwrap();
    let events = Mutex::new(Vec::new());
    let summary = execute(&project, settings, request, &CancelToken::new(), &|event| {
        events.lock().unwrap().push(event);
    })
    .unwrap_or_else(|error| panic!("{error:?}"));
    let mut tree = ResultTree::new(project.root());
    let mut command_line = String::new();
    for event in events.into_inner().unwrap() {
        match event {
            PhpUnitEvent::RunStarted(start) => command_line = start.command_line,
            PhpUnitEvent::Events(batch) => tree.apply_all(&batch),
            _ => {}
        }
    }
    Done {
        summary,
        tree,
        command_line,
    }
}

fn calculator_file() -> PathBuf {
    fixture_project().join("tests/Main/CalculatorTest.php")
}

#[test]
fn a_file_outside_the_default_suite_runs_alone() {
    let Some(script) = real_phpunit() else { return };
    // `crash` n'est pas la suite par défaut : seul le chemin la désigne.
    let done = run(
        &settings(script),
        &RunRequest::path(fixture_project().join("tests/Crash/CrashTest.php")),
    );
    assert_eq!(done.summary.counts.started, 2, "{}", done.command_line);
    assert!(done
        .command_line
        .trim_end_matches('"')
        .ends_with("CrashTest.php"));
}

#[test]
fn a_directory_runs_the_tests_it_contains() {
    let Some(script) = real_phpunit() else { return };
    let done = run(
        &settings(script),
        &RunRequest::path(fixture_project().join("tests/Main")),
    );
    assert_eq!(done.summary.counts.started, 13);
}

#[test]
fn a_class_runs_all_its_tests_a_method_only_itself_and_a_prefix_nothing() {
    let Some(script) = real_phpunit() else { return };
    let settings = settings(script);
    let done = run(&settings, &RunRequest::class(calculator_file(), CLASS));
    assert_eq!(done.summary.counts.started, 13);

    let done = run(
        &settings,
        &RunRequest::method(calculator_file(), CLASS, "testAdditionPasses"),
    );
    assert_eq!(
        (done.summary.counts.started, done.summary.counts.passed),
        (1, 1)
    );

    // Les trois jeux de données d'une méthode.
    let done = run(
        &settings,
        &RunRequest::method(calculator_file(), CLASS, "testAdditionWithDataSet"),
    );
    assert_eq!(done.summary.counts.started, 3);

    // `testAddition` est le début de deux autres méthodes : le filtre est ancré.
    let done = run(
        &settings,
        &RunRequest::method(calculator_file(), CLASS, "testAddition"),
    );
    assert_eq!(done.summary.counts.started, 0, "{}", done.command_line);
}

#[test]
fn data_sets_with_trap_names_are_selected_one_by_one() {
    let Some(script) = real_phpunit() else { return };
    let settings = settings(script);
    let file = calculator_file();
    for (name, state) in [
        ("zéro plus zéro", TestState::Passed),
        (
            "l'apostrophe | la barre [et] les crochets",
            TestState::Passed,
        ),
        ("échoue", TestState::Failed),
    ] {
        let request = RunRequest::data_set(
            &file,
            CLASS,
            "testAdditionWithDataSet",
            &DataSetKey::Name(name.into()),
        );
        let done = run(&settings, &request);
        assert_eq!(
            done.summary.counts.started, 1,
            "{name} : {}",
            done.command_line
        );
        let leaf = done
            .tree
            .iter()
            .find(|node| node.kind == NodeKind::DataSet)
            .unwrap();
        assert_eq!(leaf.label, name);
        assert_eq!(leaf.state, state);
    }
    // Un jeu qui n'existe pas : rien n'est lancé, et rien d'autre à sa place.
    let request = RunRequest::data_set(
        &file,
        CLASS,
        "testAdditionWithDataSet",
        &DataSetKey::Name("zéro".into()),
    );
    assert_eq!(run(&settings, &request).summary.counts.started, 0);
}

#[test]
fn every_tricky_data_set_name_selects_exactly_itself() {
    let Some(script) = real_phpunit() else { return };
    let settings = settings(script);
    let file = fixture_project().join("tests/Tricky/TrickyNamesTest.php");
    let class = r"Fixture\Tests\TrickyNamesTest";
    let names = [
        "simple",
        "zéro plus zéro",
        "l'apostrophe | la barre [et] les crochets",
        "des \"guillemets\" doubles",
        "antislash \\ et slash /",
        "dièse #1 et arobase @x",
        "regex (a|b)+ .* ^$ {2} ?",
        ".*",
        "ligne 1\nligne 2",
        "日本語",
        "avec  deux  espaces",
    ];
    // La méthode entière : ses 11 jeux, et pas `testTricky`.
    let done = run(
        &settings,
        &RunRequest::method(&file, class, "testTrickyNames"),
    );
    assert_eq!(
        done.summary.counts.started,
        names.len(),
        "{}",
        done.command_line
    );
    let done = run(&settings, &RunRequest::method(&file, class, "testTricky"));
    assert_eq!(done.summary.counts.started, 1);

    for name in names {
        let request = RunRequest::data_set(
            &file,
            class,
            "testTrickyNames",
            &DataSetKey::Name(name.into()),
        );
        let done = run(&settings, &request);
        assert_eq!(
            (done.summary.counts.started, done.summary.counts.passed),
            (1, 1),
            "{name:?} : {}",
            done.command_line
        );
    }
}

#[test]
fn each_node_of_the_tree_relaunches_what_it_designates() {
    let Some(script) = real_phpunit() else { return };
    let settings = settings(script);
    let full = run(&settings, &RunRequest::all());
    assert_eq!(full.summary.counts.started, 13);

    // Les trois jeux de données, un par un, avec leur état d'origine.
    let sets: Vec<_> = full
        .tree
        .iter()
        .filter(|node| node.kind == NodeKind::DataSet)
        .map(|node| (node.id, node.label.clone(), node.state))
        .collect();
    assert_eq!(sets.len(), 3);
    for (id, label, state) in sets {
        let request = RunRequest::for_node(&full.tree, id, None).unwrap();
        let done = run(&settings, &request);
        assert_eq!(
            done.summary.counts.started, 1,
            "{label} : {}",
            done.command_line
        );
        let again = done
            .tree
            .iter()
            .find(|node| node.kind == NodeKind::DataSet)
            .unwrap();
        assert_eq!((again.label.as_str(), again.state), (label.as_str(), state));
    }

    // La méthode à jeux de données.
    let method = full
        .tree
        .iter()
        .find(|node| {
            node.kind == NodeKind::Method && node.name.ends_with("testAdditionWithDataSet")
        })
        .unwrap()
        .id;
    let request = RunRequest::for_node(&full.tree, method, None).unwrap();
    assert_eq!(run(&settings, &request).summary.counts.started, 3);

    // La classe, la suite et la racine.
    let class = full
        .tree
        .iter()
        .find(|node| node.kind == NodeKind::Class)
        .unwrap()
        .id;
    let request = RunRequest::for_node(&full.tree, class, None).unwrap();
    assert_eq!(run(&settings, &request).summary.counts.started, 13);
    let suite = full
        .tree
        .iter()
        .find(|node| node.kind == NodeKind::Suite)
        .unwrap()
        .id;
    let request = RunRequest::for_node(&full.tree, suite, None).unwrap();
    assert_eq!(run(&settings, &request).summary.counts.started, 13);
}

#[test]
fn rerun_failures_runs_exactly_the_failed_and_errored_tests() {
    let Some(script) = real_phpunit() else { return };
    let settings = settings(script);
    let full = run(&settings, &RunRequest::all());
    let counts = full.summary.counts.clone();
    assert_eq!((counts.failed, counts.errors), (3, 1));

    let request = RunRequest::rerun_failures(&full.tree, Some(&RunRequest::all())).unwrap();
    let again = run(&settings, &request);
    assert_eq!(again.summary.counts.started, 4, "{}", again.command_line);
    assert_eq!(
        (again.summary.counts.failed, again.summary.counts.errors),
        (3, 1)
    );
    // Les mêmes tests, aux mêmes états.
    let names = |tree: &ResultTree| {
        let mut list: Vec<_> = tree
            .iter()
            .filter(|node| node.is_test && node.state.is_failure())
            .map(|node| node.name.clone())
            .collect();
        list.sort();
        list
    };
    assert_eq!(names(&again.tree), names(&full.tree));
}

#[test]
fn rerun_failures_of_a_run_outside_the_default_suite_keeps_its_base() {
    let Some(script) = real_phpunit() else { return };
    let settings = settings(script);
    let previous = RunRequest::test_suite("crash");
    let first = run(&settings, &previous);
    // `exit(1)` : testExitsTheProcess est en erreur.
    assert_eq!(first.summary.counts.errors, 1);
    let request = RunRequest::rerun_failures(&first.tree, Some(&previous)).unwrap();
    let again = run(&settings, &request);
    assert_eq!(again.summary.counts.started, 1, "{}", again.command_line);
    assert_eq!(again.summary.counts.errors, 1);
}

#[test]
fn settings_extra_args_and_env_reach_the_process_and_the_request_wins() {
    let Some(script) = real_phpunit() else { return };
    let mut settings = settings(script);
    settings.env.insert("APP_ENV".into(), "test".into());
    settings
        .env
        .insert("FROM_SETTINGS".into(), "réglage".into());
    settings
        .env
        .insert("OVERRIDDEN".into(), "du réglage".into());
    let mut request = RunRequest::test_suite("env");
    request.env.push(("FROM_REQUEST".into(), "demande".into()));
    request
        .env
        .push(("OVERRIDDEN".into(), "de la demande".into()));
    let done = run(&settings, &request);
    assert_eq!(done.summary.counts.passed, 1);
    let console = done
        .tree
        .iter()
        .find(|node| node.name == "testShowsTheEnvironment")
        .unwrap()
        .console_text();
    assert_eq!(
        console,
        "APP_ENV=test\nFROM_SETTINGS=réglage\nFROM_REQUEST=demande\nOVERRIDDEN=de la demande\n"
    );

    // Sans réglage ni variable : rien n'est défini (l'environnement de l'IDE
    // n'est pas touché).
    let bare = run(
        &PhpUnitSettings {
            phpunit_path: settings.phpunit_path.clone(),
            ..Default::default()
        },
        &RunRequest::test_suite("env"),
    );
    assert_eq!(
        bare.tree
            .iter()
            .find(|node| node.name == "testShowsTheEnvironment")
            .unwrap()
            .console_text(),
        "APP_ENV=(absent)\nFROM_SETTINGS=(absent)\nFROM_REQUEST=(absent)\nOVERRIDDEN=(absent)\n"
    );

    // `extra_args` : l'option arrive à PHPUnit (`--stop-on-failure` arrête à la
    // première erreur ; les 13 tests ne sont pas tous lancés).
    let mut with_args = PhpUnitSettings {
        phpunit_path: settings.phpunit_path.clone(),
        ..Default::default()
    };
    with_args.extra_args = vec!["--stop-on-failure".into()];
    let stopped = run(&with_args, &RunRequest::all());
    assert!(stopped.summary.counts.started < 13);
    assert!(stopped
        .command_line
        .contains("--teamcity --stop-on-failure"));
}

#[test]
fn the_preview_is_the_command_that_is_written_at_the_top_of_the_console() {
    let Some(script) = real_phpunit() else { return };
    let mut settings = settings(script);
    settings.extra_args = vec!["--group".into(), "un groupe".into()];
    settings.env.insert("APP_ENV".into(), "test".into());
    let request = RunRequest::method(calculator_file(), CLASS, "testAdditionPasses");

    let preview = preview_command(&fixture_project(), &settings, &request).unwrap();
    assert_eq!(preview.env, [("APP_ENV".to_owned(), "test".to_owned())]);
    assert_eq!(preview.working_dir, fixture_project());

    let mut real = request.clone();
    real.extra_args = Vec::new();
    // `--group` filtre sur un groupe qui n'existe pas : rien ne tourne, mais la
    // ligne de commande est la même.
    let done = run(&settings, &real);
    assert_eq!(preview.command_line, done.command_line);
    assert!(preview.command_line.contains(r#"--group "un groupe""#));
    assert!(preview
        .command_line
        .trim_end_matches('"')
        .ends_with("CalculatorTest.php"));
}
