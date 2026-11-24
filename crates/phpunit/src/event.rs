//! Les mutations de l'arbre de résultats : ce que l'analyseur TeamCity produit,
//! ce que le service publie par lots et ce que [`crate::ResultTree::apply`] consomme.

use std::time::Duration;

use julie_core::process::Stream;

use crate::detect::PhpUnitVersion;
use crate::model::TestState;
use crate::trace::unexport;

/// Attendu et obtenu d'un échec de comparaison, tels que PHPUnit les exporte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    /// Valeur « exportée » : une chaîne est entre apostrophes.
    pub expected: String,
    pub actual: String,
}

impl Comparison {
    /// Texte de l'attendu, prêt pour un diff (apostrophes et `\n` d'export retirés
    /// pour une chaîne ; toute autre valeur telle quelle).
    pub fn expected_text(&self) -> String {
        unexport(&self.expected)
    }

    /// Texte de l'obtenu, prêt pour un diff.
    pub fn actual_text(&self) -> String {
        unexport(&self.actual)
    }
}

/// Pourquoi une exécution s'est arrêtée avant la fin du rapport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// PHP s'est arrêté au milieu de la suite (`exit()`, erreur fatale, mémoire
    /// épuisée, PHPUnit qui ne démarre pas) ; code de sortie s'il y en a un.
    ProcessExited { exit_code: Option<i32> },
    /// L'utilisatrice a arrêté l'exécution.
    Stopped,
    /// Le délai maximal est dépassé.
    TimedOut,
}

/// Le résumé que PHPUnit écrit lui-même en fin de sortie
/// (`Tests: 13, Assertions: 10, Errors: 1, Failures: 3, ...` ou
/// `OK (5 tests, 5 assertions)`). Sert de témoin pour vérifier les compteurs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportedSummary {
    pub tests: u32,
    pub assertions: u32,
    pub errors: u32,
    pub failures: u32,
    pub warnings: u32,
    pub skipped: u32,
    pub incomplete: u32,
    pub risky: u32,
    pub deprecations: u32,
    pub notices: u32,
    /// Ligne `OK (...)` : tout a réussi, rien d'ignoré.
    pub ok: bool,
}

/// Un changement à appliquer à l'arbre de résultats.
#[derive(Debug, Clone, PartialEq)]
pub enum TestEvent {
    /// La ligne de commande lancée, affichée en tête de la console.
    Command(String),
    /// Version de PHPUnit lue sur la ligne d'en-tête de sa sortie.
    Version(PhpUnitVersion),
    /// Nombre de tests annoncé par PHPUnit (`testCount`).
    TestCount(usize),
    SuiteStarted {
        name: String,
        /// `php_qn://fichier::\Classe::méthode`.
        location_hint: Option<String>,
    },
    SuiteFinished {
        name: String,
    },
    TestStarted {
        name: String,
        location_hint: Option<String>,
    },
    TestFinished {
        name: String,
        duration: Option<Duration>,
    },
    /// `testFailed` : `state` vaut `Failed`, `Error` ou `Risky` (PHPUnit 9 les
    /// émet tous trois sous le même message ; l'analyseur les distingue d'après le
    /// message).
    TestFailed {
        name: String,
        state: TestState,
        message: String,
        details: String,
        duration: Option<Duration>,
        comparison: Option<Comparison>,
    },
    /// `testIgnored` : `state` vaut `Skipped` ou `Incomplete`.
    TestIgnored {
        name: String,
        state: TestState,
        message: String,
        details: String,
        duration: Option<Duration>,
    },
    /// Une ligne de sortie qui n'est pas un message TeamCity : jamais perdue,
    /// rattachée au test en cours, sinon à la racine.
    Output {
        stream: Stream,
        text: String,
    },
    /// Le résumé écrit par PHPUnit en fin de sortie.
    Summary(ReportedSummary),
    /// L'exécution s'est arrêtée avant la fin : les tests en cours sont conclus.
    Aborted(AbortReason),
    /// Plus rien ne viendra : l'arbre est figé.
    Ended,
}
