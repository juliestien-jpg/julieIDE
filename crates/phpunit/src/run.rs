//! Ce qu'on lance et comment : plan de lancement, demande d'exécution, commande,
//! résumé de fin.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use julie_core::process::Command;

use crate::event::ReportedSummary;
use crate::model::Counts;

/// Tout ce qu'il faut pour lancer : rendu par [`crate::Detection::plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    /// Racine du projet : dossier de travail par défaut.
    pub root: PathBuf,
    pub php: PathBuf,
    /// Script PHP ou phar de PHPUnit.
    pub phpunit: PathBuf,
    pub configuration: PathBuf,
}

/// Ce qu'on exécute. En v0 l'interface n'utilise que `All` ; les autres portées
/// servent aux essais et préparent « lancer ceci » de la v1.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RunScope {
    /// La suite par défaut de la configuration.
    #[default]
    All,
    /// Une suite de `phpunit.xml` (`--testsuite`).
    TestSuite(String),
    /// Un filtre de PHPUnit (`--filter`, expression régulière) : une classe, une
    /// méthode ou un jeu de données.
    Filter(String),
}

/// Une demande d'exécution. `RunRequest::default()` lance toute la suite avec la
/// configuration du projet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunRequest {
    pub scope: RunScope,
    /// Options PHPUnit supplémentaires, ajoutées telles quelles à la fin.
    pub extra_args: Vec<String>,
    /// Variables d'environnement en plus de celles de l'IDE.
    pub env: Vec<(String, String)>,
    /// Dossier de travail ; par défaut la racine du projet.
    pub working_dir: Option<PathBuf>,
    /// Remplace le délai du réglage `timeout_seconds`.
    pub timeout: Option<Duration>,
}

impl RunRequest {
    /// Toute la suite.
    pub fn all() -> Self {
        Self::default()
    }

    pub fn test_suite(name: impl Into<String>) -> Self {
        Self {
            scope: RunScope::TestSuite(name.into()),
            ..Self::default()
        }
    }

    pub fn filter(pattern: impl Into<String>) -> Self {
        Self {
            scope: RunScope::Filter(pattern.into()),
            ..Self::default()
        }
    }
}

/// Arguments donnés à PHP : `<phpunit> --configuration <xml> --teamcity [portée]
/// [options]`.
pub fn arguments(plan: &LaunchPlan, request: &RunRequest) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        plan.phpunit.clone().into_os_string(),
        "--configuration".into(),
        plan.configuration.clone().into_os_string(),
        "--teamcity".into(),
    ];
    match &request.scope {
        RunScope::All => {}
        RunScope::TestSuite(name) => {
            args.push("--testsuite".into());
            args.push(name.into());
        }
        RunScope::Filter(pattern) => {
            args.push("--filter".into());
            args.push(pattern.into());
        }
    }
    args.extend(request.extra_args.iter().map(OsString::from));
    args
}

/// La commande à lancer.
pub fn command(plan: &LaunchPlan, request: &RunRequest) -> Command {
    let mut command = Command::new(&plan.php)
        .args(arguments(plan, request))
        .current_dir(request.working_dir.as_ref().unwrap_or(&plan.root));
    for (key, value) in &request.env {
        command = command.env(key, value);
    }
    command
}

/// La commande écrite pour l'œil (en tête de la console) : les arguments qui
/// contiennent des espaces sont entre guillemets.
pub fn command_line(plan: &LaunchPlan, request: &RunRequest) -> String {
    let mut parts = vec![quote(&plan.php.to_string_lossy())];
    parts.extend(
        arguments(plan, request)
            .iter()
            .map(|arg| quote(&arg.to_string_lossy())),
    );
    parts.join(" ")
}

fn quote(arg: &str) -> String {
    if arg.is_empty() || arg.contains([' ', '\t', '"']) {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_owned()
    }
}

/// Comment l'exécution s'est terminée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// PHPUnit est allé au bout de son rapport (même avec des échecs).
    Completed,
    /// Le processus s'est arrêté avant la fin du rapport : `exit()`, erreur
    /// fatale, PHPUnit qui ne démarre pas.
    Aborted { exit_code: Option<i32> },
    /// Arrêté sur demande.
    Stopped,
    /// Arrêté parce que le délai maximal était dépassé.
    TimedOut,
}

/// Bilan d'une exécution, publié en dernier (`PhpUnitEvent::RunFinished`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub outcome: RunOutcome,
    /// Code de sortie de PHP, s'il s'est terminé de lui-même. Non nul avec des
    /// échecs, mais aussi parfois sans (avertissements) : à afficher, pas à
    /// interpréter.
    pub exit_code: Option<i32>,
    pub counts: Counts,
    /// Durée réelle, de l'appel à la fin du processus.
    pub duration: Duration,
    /// Le résumé écrit par PHPUnit lui-même, s'il est arrivé.
    pub reported: Option<ReportedSummary>,
}

impl RunSummary {
    /// Allé au bout, sans échec ni erreur. « Aucun test exécuté » se lit sur
    /// `counts.started == 0`.
    pub fn is_green(&self) -> bool {
        self.outcome == RunOutcome::Completed && self.counts.failures() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> LaunchPlan {
        LaunchPlan {
            root: PathBuf::from(r"C:\Mes projets\p"),
            php: PathBuf::from(r"C:\php\php.exe"),
            phpunit: PathBuf::from(r"C:\Mes projets\p\vendor\bin\phpunit"),
            configuration: PathBuf::from(r"C:\Mes projets\p\phpunit.xml"),
        }
    }

    #[test]
    fn the_default_command_is_phpunit_configuration_teamcity() {
        let args: Vec<String> = arguments(&plan(), &RunRequest::all())
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                r"C:\Mes projets\p\vendor\bin\phpunit",
                "--configuration",
                r"C:\Mes projets\p\phpunit.xml",
                "--teamcity"
            ]
        );
        assert_eq!(
            command_line(&plan(), &RunRequest::all()),
            r#"C:\php\php.exe "C:\Mes projets\p\vendor\bin\phpunit" --configuration "C:\Mes projets\p\phpunit.xml" --teamcity"#
        );
    }

    #[test]
    fn scope_and_extra_options_come_after_teamcity() {
        let mut request = RunRequest::filter("CalculatorTest::testAdd");
        request.extra_args = vec!["--stop-on-failure".into()];
        let line = command_line(&plan(), &request);
        assert!(line.ends_with("--teamcity --filter CalculatorTest::testAdd --stop-on-failure"));
        let line = command_line(&plan(), &RunRequest::test_suite("crash"));
        assert!(line.ends_with("--teamcity --testsuite crash"));
    }
}
