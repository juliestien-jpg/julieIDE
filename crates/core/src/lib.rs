//! Noyau de JulieStorm : types communs, bus d'événements, tâches de fond,
//! processus externes, fichiers texte, projet et réglages.
//!
//! Toutes les crates métier dépendent de celle-ci et d'aucune autre.
//! Propriétaire : architecte-technique. Contrats décrits dans `docs/contrats.md`.

pub mod bus;
pub mod error;
pub mod event;
pub mod file;
pub mod process;
pub mod project;
pub mod settings;
pub mod task;

use std::path::PathBuf;

pub use bus::{Bus, Publisher};
pub use error::{Error, Result};
pub use event::{Domain, DomainEvent, Event};
pub use project::Project;
pub use settings::Settings;
pub use task::{
    CancelToken, TaskContext, TaskEvent, TaskHandle, TaskId, TaskOutcome, TaskResult, TaskState,
    Tasks,
};

/// Position dans un texte : ligne et colonne comptées à partir de zéro,
/// la colonne en caractères (pas en octets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextPosition {
    pub line: u32,
    pub column: u32,
}

impl TextPosition {
    pub fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

/// Plage dans un texte : `start` inclus, `end` exclu, avec `start <= end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}

impl TextRange {
    /// Construit la plage en remettant les bornes dans l'ordre.
    pub fn new(a: TextPosition, b: TextPosition) -> Self {
        Self {
            start: a.min(b),
            end: a.max(b),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, position: TextPosition) -> bool {
        self.start <= position && position < self.end
    }
}

/// Un endroit du projet où l'on peut sauter : résultat de recherche, trace de test,
/// ligne d'un diff. Le chemin est absolu.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub path: PathBuf,
    pub position: TextPosition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_orders_its_bounds_and_excludes_its_end() {
        let range = TextRange::new(TextPosition::new(3, 0), TextPosition::new(1, 4));
        assert_eq!(range.start, TextPosition::new(1, 4));
        assert!(range.contains(TextPosition::new(1, 4)));
        assert!(range.contains(TextPosition::new(2, 99)));
        assert!(!range.contains(TextPosition::new(3, 0)));
        assert!(!range.is_empty());
    }
}
