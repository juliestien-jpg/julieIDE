//! Rattrapage d'une distinction que PHPUnit 9 ne fait pas : un test ignoré par
//! `markTestSkipped()` et un test incomplet par `markTestIncomplete()` sont émis
//! tous deux comme `testIgnored`, avec le message de l'auteur du test.
//!
//! La première ligne de la trace est l'endroit de l'appel : si elle contient
//! `markTestIncomplete`, le test est incomplet. C'est un indice, pas une preuve
//! (un appel écrit sur plusieurs lignes, ou enveloppé dans une aide, échappe) : en
//! cas de doute le test reste « ignoré ».

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::event::TestEvent;
use crate::model::TestState;
use crate::trace::parse_trace;

/// Les fichiers déjà lus, pour ne pas relire un fichier de test par test ignoré.
#[derive(Debug, Default)]
pub struct SourceLines {
    files: HashMap<PathBuf, Option<Vec<String>>>,
}

impl SourceLines {
    fn line(&mut self, path: &Path, line: usize) -> Option<&str> {
        let lines = self.files.entry(path.to_path_buf()).or_insert_with(|| {
            std::fs::read(path).ok().map(|bytes| {
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .map(str::to_owned)
                    .collect()
            })
        });
        lines.as_ref()?.get(line).map(String::as_str)
    }
}

/// Corrige l'état d'un `TestIgnored` d'après le code source (accès disque).
/// Sans effet sur les autres événements.
pub fn refine(event: &mut TestEvent, root: &Path, sources: &mut SourceLines) {
    let TestEvent::TestIgnored { state, details, .. } = event else {
        return;
    };
    if *state != TestState::Skipped {
        return;
    }
    let first = parse_trace(details, root)
        .into_iter()
        .find_map(|entry| entry.location);
    let Some(location) = first else {
        return;
    };
    let Some(line) = sources.line(&location.path, location.position.line as usize) else {
        return;
    };
    if line.to_ascii_lowercase().contains("marktestincomplete") {
        *state = TestState::Incomplete;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn ignored(details: &str) -> TestEvent {
        TestEvent::TestIgnored {
            name: "t".into(),
            state: TestState::Skipped,
            message: "m".into(),
            details: details.into(),
            duration: Some(Duration::ZERO),
        }
    }

    #[test]
    fn the_call_line_tells_incomplete_from_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ATest.php");
        std::fs::write(
            &file,
            "<?php\n$this->markTestSkipped('x');\n$this->markTestIncomplete('y');\n",
        )
        .unwrap();
        let mut sources = SourceLines::default();

        let mut skipped = ignored(&format!(" {}:2\n ", file.display()));
        refine(&mut skipped, dir.path(), &mut sources);
        assert!(matches!(
            skipped,
            TestEvent::TestIgnored {
                state: TestState::Skipped,
                ..
            }
        ));

        let mut incomplete = ignored(&format!(" {}:3\n ", file.display()));
        refine(&mut incomplete, dir.path(), &mut sources);
        assert!(matches!(
            incomplete,
            TestEvent::TestIgnored {
                state: TestState::Incomplete,
                ..
            }
        ));
    }

    #[test]
    fn unreadable_or_absent_traces_change_nothing() {
        let mut sources = SourceLines::default();
        let mut event = ignored(" C:\\nulle\\part\\x.php:3\n ");
        refine(&mut event, Path::new("C:\\p"), &mut sources);
        assert!(matches!(
            event,
            TestEvent::TestIgnored {
                state: TestState::Skipped,
                ..
            }
        ));
        let mut event = ignored("");
        refine(&mut event, Path::new("C:\\p"), &mut sources);
        let mut other = TestEvent::Ended;
        refine(&mut other, Path::new("C:\\p"), &mut sources);
        assert_eq!(other, TestEvent::Ended);
    }
}
