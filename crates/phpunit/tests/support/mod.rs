//! Outils communs aux essais d'intégration.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use julie_core::process::Stream;
use julie_core::{Bus, Domain, Event, TaskId, TaskOutcome, TaskState};
use julie_phpunit::{PhpUnitEvent, ResultTree, TeamCityParser, TestEvent};

/// Dossier du petit projet PHP d'essai.
pub fn fixture_project() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("racine de l'espace de travail")
        .join("tests")
        .join("fixtures")
        .join("phpunit")
}

pub fn recording(name: &str) -> String {
    let path = fixture_project().join("recordings").join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Rejoue un enregistrement comme le fait `Command::run_lines` : ligne par ligne,
/// sans fin de ligne. Rend les événements dans l'ordre.
pub fn replay(text: &str) -> Vec<TestEvent> {
    let mut parser = TeamCityParser::new();
    let mut events = Vec::new();
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for line in lines {
        let line = line.strip_suffix('\r').unwrap_or(line);
        events.extend(parser.feed(Stream::Stdout, line));
    }
    events.extend(parser.finish());
    events
}

pub fn tree_of(events: &[TestEvent], root: &str) -> ResultTree {
    let mut tree = ResultTree::new(root);
    tree.apply_all(events);
    tree
}

// --- exécutions réelles ---------------------------------------------------------

/// Le PHPUnit 9.6 de la machine, utilisé en lecture seule sur le projet d'essai.
pub const LAB_PHPUNIT: &str =
    r"C:\Program Files (x86)\AgileBio\LabCollector\www\lab\vendor\bin\phpunit";

/// PHP et PHPUnit réels, ou `None` (l'essai se contente alors de le dire).
pub fn real_phpunit() -> Option<PathBuf> {
    let script = std::env::var_os("JULIE_PHPUNIT_SCRIPT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(LAB_PHPUNIT));
    let php = Path::new(julie_phpunit::DEFAULT_PHP_PATH);
    if script.is_file() && php.is_file() {
        Some(script)
    } else {
        eprintln!("essai sauté : PHP ou PHPUnit absents de cette machine");
        None
    }
}

/// Un événement du bus de PHPUnit avec son heure d'arrivée.
pub struct Received {
    pub at: Instant,
    pub event: PhpUnitEvent,
}

pub struct RunLog {
    pub events: Vec<Received>,
    pub outcome: TaskOutcome,
}

impl RunLog {
    pub fn finished(&self) -> Option<&julie_phpunit::RunSummary> {
        self.events.iter().find_map(|r| match &r.event {
            PhpUnitEvent::RunFinished(summary) => Some(summary.as_ref()),
            _ => None,
        })
    }

    /// L'arbre que construirait l'interface à partir des seuls événements publiés.
    pub fn ui_tree(&self, root: &Path) -> ResultTree {
        let mut tree = ResultTree::new(root);
        for received in &self.events {
            if let PhpUnitEvent::Events(events) = &received.event {
                tree.apply_all(events);
            }
        }
        tree
    }

    /// Heure d'arrivée du premier lot qui contient un événement qui convient.
    pub fn first_arrival(&self, wanted: impl Fn(&TestEvent) -> bool) -> Option<Instant> {
        self.events
            .iter()
            .find_map(|received| match &received.event {
                PhpUnitEvent::Events(events) if events.iter().any(&wanted) => Some(received.at),
                _ => None,
            })
    }
}

/// Lit le bus jusqu'à la fin de la tâche `task` (ou `limit` écoulé).
pub fn collect_until_finished(bus: &Bus, task: TaskId, limit: Duration) -> RunLog {
    let start = Instant::now();
    let mut events = Vec::new();
    loop {
        let remaining = limit.saturating_sub(start.elapsed());
        let Some(event) = bus.recv_timeout(remaining.max(Duration::from_millis(1))) else {
            panic!("la tâche {task:?} ne finit pas en {limit:?}");
        };
        match event {
            Event::Domain(event) if event.domain == Domain::PhpUnit && event.task == Some(task) => {
                if let Some(event) = event.get::<PhpUnitEvent>() {
                    events.push(Received {
                        at: Instant::now(),
                        event: event.clone(),
                    });
                }
            }
            Event::Task(task_event) if task_event.id == task => {
                if let TaskState::Finished(outcome) = task_event.state {
                    return RunLog { events, outcome };
                }
            }
            _ => {}
        }
    }
}
