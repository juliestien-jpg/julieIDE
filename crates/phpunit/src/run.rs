//! Ce qu'on lance et comment : plan de lancement, demande d'exécution, commande,
//! résumé de fin.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use julie_core::process::Command;

use crate::detect::{detect_files, Detection};
use crate::error::Problem;
use crate::settings::{is_valid_env_name, PhpUnitSettings};

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

/// Ce qu'on exécute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RunScope {
    /// La suite par défaut de la configuration.
    #[default]
    All,
    /// Une suite de `phpunit.xml` (`--testsuite`).
    TestSuite(String),
    /// Un motif de `--filter` donné tel quel (expression régulière de PHPUnit).
    Filter(String),
    /// Une partie de la suite : fichier, dossier, classe, méthode, jeu de données,
    /// ou les tests en échec (voir [`crate::select`]).
    Selection(Selection),
}

/// Où PHPUnit va chercher les tests d'une [`Selection`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Base {
    /// La suite par défaut de la configuration.
    #[default]
    Default,
    /// Une suite de `phpunit.xml`.
    TestSuite(String),
    /// Un fichier ou un dossier, passé en dernier argument à PHPUnit (il remplace
    /// les suites de la configuration).
    Path(PathBuf),
}

/// Une partie de la suite : une base, éventuellement restreinte par un filtre.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub base: Base,
    /// Expression régulière **déjà délimitée** (`/^…$/s`), telle que la donne
    /// [`crate::select`]. Absente : tout ce que la base contient.
    pub filter: Option<String>,
}

/// Une demande d'exécution. `RunRequest::default()` lance toute la suite avec la
/// configuration du projet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunRequest {
    pub scope: RunScope,
    /// Options PHPUnit supplémentaires de cette demande, ajoutées après celles du
    /// réglage `extra_args`.
    pub extra_args: Vec<String>,
    /// Variables d'environnement en plus de celles de l'IDE et du réglage
    /// `[phpunit.env]` (elles l'emportent sur lui).
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

    /// La demande telle qu'elle sera lancée : les options et variables du réglage
    /// d'abord, celles de la demande ensuite (qui l'emportent sur les variables du
    /// réglage de même nom). Les noms de variables invalides sont écartés.
    /// [`crate::execute`] et [`crate::preview_command`] l'appliquent ; ne pas
    /// l'appliquer deux fois.
    pub fn with_settings(&self, settings: &PhpUnitSettings) -> RunRequest {
        let mut request = self.clone();
        request.extra_args = settings
            .extra_args
            .iter()
            .chain(&self.extra_args)
            .cloned()
            .collect();
        let mut env: Vec<(String, String)> = Vec::new();
        let from_settings = settings.env.iter().map(|(k, v)| (k.clone(), v.clone()));
        for (key, value) in from_settings.chain(self.env.iter().cloned()) {
            if !is_valid_env_name(&key) {
                continue;
            }
            let same = |other: &str| {
                if cfg!(windows) {
                    other.eq_ignore_ascii_case(&key)
                } else {
                    other == key
                }
            };
            env.retain(|(existing, _)| !same(existing));
            env.push((key, value));
        }
        request.env = env;
        request
    }
}

/// Arguments donnés à PHP : `<phpunit> --configuration <xml> --teamcity [--testsuite
/// <suite>] [--filter <motif>] [options] [chemin]`. Le chemin vient en dernier :
/// PHPUnit cesse de lire les options au premier argument qui n'en est pas une.
pub fn arguments(plan: &LaunchPlan, request: &RunRequest) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        plan.phpunit.clone().into_os_string(),
        "--configuration".into(),
        plan.configuration.clone().into_os_string(),
        "--teamcity".into(),
    ];
    let mut path: Option<&PathBuf> = None;
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
        RunScope::Selection(selection) => {
            match &selection.base {
                Base::Default => {}
                Base::TestSuite(name) => {
                    args.push("--testsuite".into());
                    args.push(name.into());
                }
                Base::Path(target) => path = Some(target),
            }
            if let Some(pattern) = &selection.filter {
                args.push("--filter".into());
                args.push(pattern.into());
            }
        }
    }
    args.extend(request.extra_args.iter().map(OsString::from));
    if let Some(path) = path {
        args.push(path.clone().into_os_string());
    }
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
    if arg.is_empty() || arg.contains([' ', '\t', '"', '\n', '\r']) {
        format!(
            "\"{}\"",
            arg.replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        )
    } else {
        arg.to_owned()
    }
}

/// La commande exacte d'une exécution, prête à afficher (fenêtre de réglages) ou à
/// comparer à ce que l'on voit en tête de la console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPreview {
    pub plan: LaunchPlan,
    /// Écrite comme en tête de la console : `"C:\…\php.exe" "…\phpunit.phar"
    /// --configuration "…\phpunit.xml" --teamcity …`.
    pub command_line: String,
    /// Les arguments donnés à PHP, un par élément (le premier est le script).
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    /// Variables ajoutées à l'environnement de l'IDE (réglage puis demande).
    pub env: Vec<(String, String)>,
}

/// La commande qui serait lancée pour `request` avec ces réglages, **sans rien
/// lancer** : seulement des accès disque (`is_file`), pas de processus.
/// `Err` : le premier [`Problem`] de la détection (PHP, PHPUnit, configuration).
pub fn preview_command(
    root: &Path,
    settings: &PhpUnitSettings,
    request: &RunRequest,
) -> Result<CommandPreview, Problem> {
    let detection = detect_files(root, settings);
    preview_with(&detection, settings, request)
}

/// Comme [`preview_command`], à partir d'une détection déjà faite.
pub fn preview_with(
    detection: &Detection,
    settings: &PhpUnitSettings,
    request: &RunRequest,
) -> Result<CommandPreview, Problem> {
    let plan = detection.plan()?;
    let effective = request.with_settings(settings);
    Ok(CommandPreview {
        command_line: command_line(&plan, &effective),
        args: arguments(&plan, &effective)
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
        working_dir: effective
            .working_dir
            .clone()
            .unwrap_or_else(|| plan.root.clone()),
        env: effective.env,
        plan,
    })
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

    #[test]
    fn settings_options_come_first_and_the_path_comes_last() {
        let settings = PhpUnitSettings {
            extra_args: vec!["--stop-on-failure".into()],
            ..Default::default()
        };
        let mut request = RunRequest::path(r"C:\Mes projets\p\tests\FooTest.php");
        request.extra_args = vec!["--no-coverage".into()];
        let line = command_line(&plan(), &request.with_settings(&settings));
        assert!(
            line.ends_with(
                r#"--teamcity --stop-on-failure --no-coverage "C:\Mes projets\p\tests\FooTest.php""#
            ),
            "{line}"
        );
        // Filtre et chemin ensemble : le filtre avant les options, le chemin après.
        let request = RunRequest::method(r"C:\p\FooTest.php", r"A\FooTest", "testX");
        let line = command_line(&plan(), &request);
        assert!(
            line.contains(r#"--filter "/^A\\FooTest::testX(?: with data set .*)?$/s""#),
            "{line}"
        );
        assert!(line.ends_with(r"C:\p\FooTest.php"));
    }

    #[test]
    fn a_filter_with_spaces_or_newlines_is_shown_quoted() {
        let request = RunRequest::filter("a b\nc\"d");
        let line = command_line(&plan(), &request);
        assert!(line.ends_with(r#"--filter "a b\nc\"d""#), "{line}");
    }

    #[test]
    fn request_env_wins_over_settings_env_and_bad_names_are_dropped() {
        let mut settings = PhpUnitSettings::default();
        settings.env.insert("A".into(), "réglage".into());
        settings.env.insert("B".into(), "réglage".into());
        settings.env.insert("MAUVAIS=NOM".into(), "x".into());
        let mut request = RunRequest::all();
        request.env = vec![("B".into(), "demande".into()), (String::new(), "y".into())];
        let effective = request.with_settings(&settings);
        assert_eq!(
            effective.env,
            [
                ("A".to_owned(), "réglage".to_owned()),
                ("B".to_owned(), "demande".to_owned())
            ]
        );
        if cfg!(windows) {
            request.env = vec![("a".into(), "minuscule".into())];
            let effective = request.with_settings(&settings);
            assert_eq!(
                effective
                    .env
                    .iter()
                    .filter(|(k, _)| k.eq_ignore_ascii_case("a"))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn the_preview_applies_the_settings_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("tools/phpunit")).unwrap();
        std::fs::write(root.join("tools/phpunit/phpunit.phar"), "x").unwrap();
        std::fs::write(root.join("phpunit.xml"), "<phpunit/>").unwrap();
        let php = root.join("php.exe");
        std::fs::write(&php, "x").unwrap();
        let mut settings = PhpUnitSettings {
            php_path: Some(php.clone()),
            extra_args: vec!["--stop-on-failure".into()],
            ..Default::default()
        };
        settings.env.insert("APP_ENV".into(), "test".into());
        let preview = preview_command(root, &settings, &RunRequest::all()).unwrap();
        assert_eq!(
            preview.plan.phpunit,
            root.join("tools/phpunit/phpunit.phar")
        );
        assert_eq!(
            preview.args.last().map(String::as_str),
            Some("--stop-on-failure")
        );
        assert_eq!(
            preview
                .args
                .iter()
                .filter(|a| *a == "--stop-on-failure")
                .count(),
            1
        );
        assert_eq!(preview.env, [("APP_ENV".to_owned(), "test".to_owned())]);
        assert_eq!(preview.working_dir, root);
        assert!(preview.command_line.contains("--configuration"));
        assert!(preview
            .command_line
            .ends_with("--teamcity --stop-on-failure"));

        // Rien à lancer : le premier problème, comme la détection.
        std::fs::remove_file(root.join("phpunit.xml")).unwrap();
        assert!(matches!(
            preview_command(root, &settings, &RunRequest::all()),
            Err(Problem::NoConfiguration { .. })
        ));
    }
}
