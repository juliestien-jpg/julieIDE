//! Détection de PHP, de PHPUnit et de la configuration d'un projet.
//!
//! Ordre de recherche :
//!
//! - **PHP** : réglage `php_path`, sinon `php` dans le `PATH`, sinon
//!   [`DEFAULT_PHP_PATH`]. Un réglage qui désigne un fichier absent est une erreur
//!   ([`Problem::PhpPathInvalid`]), jamais un repli silencieux sur un autre PHP.
//! - **PHPUnit** : réglage `phpunit_path`, sinon `vendor/bin/phpunit`,
//!   `vendor/phpunit/phpunit/phpunit`, `vendor/bin/phpunit.phar`, `phpunit.phar`,
//!   `tools/phpunit.phar`. On lance toujours `php <script>`, jamais le `.bat`.
//! - **Configuration** : réglage `configuration`, sinon `phpunit.xml`,
//!   `phpunit.xml.dist`, `phpunit.dist.xml` à la racine du projet.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use julie_core::process::Command;
use julie_core::CancelToken;

use crate::error::Problem;
use crate::settings::PhpUnitSettings;

/// PHP de la machine de développement (LabCollector), dernier recours.
pub const DEFAULT_PHP_PATH: &str = r"C:\Program Files (x86)\AgileBio\LabCollector\php\php.exe";

/// D'où vient le PHP retenu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhpSource {
    Settings,
    Path,
    DefaultInstall,
}

/// Un PHP trouvé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpInfo {
    pub path: PathBuf,
    /// `8.5.13`, lu sur `php --version` (absent si la sonde n'a pas été faite).
    pub version: Option<String>,
    pub source: PhpSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhpUnitKind {
    /// Script PHP (`vendor/bin/phpunit`).
    Script,
    Phar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhpUnitSource {
    Settings,
    VendorBin,
    VendorPackage,
    Phar,
}

/// Un PHPUnit trouvé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpUnitInfo {
    pub path: PathBuf,
    pub kind: PhpUnitKind,
    pub version: Option<PhpUnitVersion>,
    pub source: PhpUnitSource,
}

/// Version de PHPUnit (`9.6.34`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhpUnitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// Le texte tel que PHPUnit l'écrit (`9.6.34`, `11.0.0-dev`).
    pub text: String,
}

/// Résultat de la détection. Chaque élément est un `Result` : l'interface peut
/// montrer ce qui va et ce qui manque, sans perdre le reste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub root: PathBuf,
    pub php: Result<PhpInfo, Problem>,
    pub phpunit: Result<PhpUnitInfo, Problem>,
    pub configuration: Result<PathBuf, Problem>,
}

impl Detection {
    /// Tout est là : on peut lancer.
    pub fn is_ready(&self) -> bool {
        self.php.is_ok() && self.phpunit.is_ok() && self.configuration.is_ok()
    }

    /// Les problèmes, dans l'ordre PHP, PHPUnit, configuration.
    pub fn problems(&self) -> Vec<&Problem> {
        let mut problems = Vec::new();
        if let Err(problem) = &self.php {
            problems.push(problem);
        }
        if let Err(problem) = &self.phpunit {
            problems.push(problem);
        }
        if let Err(problem) = &self.configuration {
            problems.push(problem);
        }
        problems
    }

    /// Ce qu'il faut pour lancer, ou le premier problème.
    pub fn plan(&self) -> Result<crate::run::LaunchPlan, Problem> {
        Ok(crate::run::LaunchPlan {
            root: self.root.clone(),
            php: self.php.clone()?.path,
            phpunit: self.phpunit.clone()?.path,
            configuration: self.configuration.clone()?,
        })
    }
}

/// Détecte PHP, PHPUnit et la configuration de `root`.
///
/// Avec `probe_versions`, lance `php --version` et `php <phpunit> --version`
/// (deux processus, quelques centaines de millisecondes) : **bloquant**, à faire
/// dans une tâche. Sans, seuls des accès disque ont lieu et les versions restent
/// `None`.
///
/// Rend `Err` seulement pour l'annulation.
pub fn detect(
    root: &Path,
    settings: &PhpUnitSettings,
    probe_versions: bool,
    cancel: &CancelToken,
) -> julie_core::Result<Detection> {
    cancel.check()?;
    let path_var = std::env::var_os("PATH");
    let mut php = locate_php(
        settings,
        root,
        path_var.as_deref(),
        Path::new(DEFAULT_PHP_PATH),
    );
    let mut phpunit = locate_phpunit(settings, root);
    let configuration = locate_configuration(settings, root);

    if probe_versions {
        if let Ok(info) = &mut php {
            match probe_php_version(&info.path, cancel) {
                Ok(version) => info.version = version,
                Err(Probe::Cancelled) => return Err(julie_core::Error::Cancelled),
                Err(Probe::Failed(reason)) => {
                    php = Err(Problem::PhpNotRunnable {
                        path: info.path.clone(),
                        reason,
                    });
                }
            }
        }
        if let (Ok(php_info), Ok(unit)) = (&php, &mut phpunit) {
            match probe_phpunit_version(&php_info.path, &unit.path, root, cancel) {
                Ok(version) => unit.version = version,
                Err(Probe::Cancelled) => return Err(julie_core::Error::Cancelled),
                // PHPUnit qui ne répond pas à --version : on ne bloque pas, la
                // version reste inconnue et l'exécution dira ce qui ne va pas.
                Err(Probe::Failed(_)) => {}
            }
        }
    }

    Ok(Detection {
        root: root.to_path_buf(),
        php,
        phpunit,
        configuration,
    })
}

/// Cherche PHP. `path_var` est la valeur de la variable `PATH` (injectée pour les
/// tests) ; `fallback` l'emplacement de dernier recours.
pub fn locate_php(
    settings: &PhpUnitSettings,
    root: &Path,
    path_var: Option<&OsStr>,
    fallback: &Path,
) -> Result<PhpInfo, Problem> {
    let found = |path: PathBuf, source| PhpInfo {
        path,
        version: None,
        source,
    };
    if let Some(configured) = settings
        .php_path
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    {
        let is_bare_name = configured.components().count() == 1 && !configured.is_absolute();
        if is_bare_name {
            return search_path(configured, path_var)
                .map(|path| found(path, PhpSource::Settings))
                .ok_or_else(|| Problem::PhpPathInvalid(configured.clone()));
        }
        let resolved = resolve(root, configured);
        return if resolved.is_file() {
            Ok(found(resolved, PhpSource::Settings))
        } else {
            Err(Problem::PhpPathInvalid(configured.clone()))
        };
    }
    if let Some(path) = search_path(Path::new("php"), path_var) {
        return Ok(found(path, PhpSource::Path));
    }
    if fallback.is_file() {
        return Ok(found(fallback.to_path_buf(), PhpSource::DefaultInstall));
    }
    Err(Problem::PhpNotFound {
        tried: vec![PathBuf::from("php"), fallback.to_path_buf()],
    })
}

/// Cherche PHPUnit dans le projet.
pub fn locate_phpunit(settings: &PhpUnitSettings, root: &Path) -> Result<PhpUnitInfo, Problem> {
    if let Some(configured) = settings
        .phpunit_path
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    {
        let mut resolved = resolve(root, configured);
        let is_launcher = resolved
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd"));
        if is_launcher {
            resolved.set_extension("");
        }
        return if resolved.is_file() {
            Ok(PhpUnitInfo {
                kind: kind_of(&resolved),
                path: resolved,
                version: None,
                source: PhpUnitSource::Settings,
            })
        } else {
            Err(Problem::PhpUnitPathInvalid(configured.clone()))
        };
    }
    let candidates = [
        ("vendor/bin/phpunit", PhpUnitSource::VendorBin),
        (
            "vendor/phpunit/phpunit/phpunit",
            PhpUnitSource::VendorPackage,
        ),
        ("vendor/bin/phpunit.phar", PhpUnitSource::Phar),
        ("phpunit.phar", PhpUnitSource::Phar),
        ("tools/phpunit.phar", PhpUnitSource::Phar),
    ];
    let mut searched = Vec::new();
    for (relative, source) in candidates {
        let path = root.join(relative);
        if path.is_file() {
            return Ok(PhpUnitInfo {
                kind: kind_of(&path),
                path,
                version: None,
                source,
            });
        }
        searched.push(path);
    }
    Err(Problem::PhpUnitNotFound { searched })
}

/// Cherche le fichier de configuration.
pub fn locate_configuration(settings: &PhpUnitSettings, root: &Path) -> Result<PathBuf, Problem> {
    if let Some(configured) = settings
        .configuration
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
    {
        let resolved = resolve(root, configured);
        return if resolved.is_file() {
            Ok(resolved)
        } else {
            Err(Problem::ConfigurationPathInvalid(configured.clone()))
        };
    }
    ["phpunit.xml", "phpunit.xml.dist", "phpunit.dist.xml"]
        .into_iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| Problem::NoConfiguration {
            root: root.to_path_buf(),
        })
}

fn kind_of(path: &Path) -> PhpUnitKind {
    if path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("phar"))
    {
        PhpUnitKind::Phar
    } else {
        PhpUnitKind::Script
    }
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Cherche `name` dans les dossiers de `path_var` (`.exe` en plus sous Windows).
fn search_path(name: &Path, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let plain = dir.join(name);
        if cfg!(windows) {
            let mut with_ext = plain.clone().into_os_string();
            with_ext.push(".exe");
            let with_ext = PathBuf::from(with_ext);
            if with_ext.is_file() {
                return Some(with_ext);
            }
            if plain.extension().is_some() && plain.is_file() {
                return Some(plain);
            }
        } else if plain.is_file() {
            return Some(plain);
        }
    }
    None
}

enum Probe {
    Cancelled,
    Failed(String),
}

fn probe_php_version(php: &Path, cancel: &CancelToken) -> Result<Option<String>, Probe> {
    let command = Command::new(php).arg("--version");
    let output = run_probe(&command, cancel)?;
    Ok(parse_php_version(&output))
}

fn probe_phpunit_version(
    php: &Path,
    phpunit: &Path,
    root: &Path,
    cancel: &CancelToken,
) -> Result<Option<PhpUnitVersion>, Probe> {
    let command = Command::new(php)
        .arg(phpunit)
        .arg("--version")
        .current_dir(root);
    let output = run_probe(&command, cancel)?;
    Ok(output.lines().find_map(parse_phpunit_version))
}

fn run_probe(command: &Command, cancel: &CancelToken) -> Result<String, Probe> {
    match command.run(cancel) {
        Ok(output) if output.status.success() => Ok(output.stdout_text()),
        Ok(output) => Err(Probe::Failed(format!(
            "code de sortie {:?} : {}",
            output.status.code,
            output.stderr_text().trim()
        ))),
        Err(error) if error.is_cancelled() => Err(Probe::Cancelled),
        Err(error) => Err(Probe::Failed(error.to_string())),
    }
}

/// `PHP 8.5.13 (cli) (built: …)` -> `8.5.13`. Les lignes de bannière que d'autres
/// extensions ajoutent après (ionCube…) sont ignorées.
pub fn parse_php_version(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("PHP "))
        .and_then(|rest| rest.split_whitespace().next())
        .filter(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_owned)
}

/// `PHPUnit 9.6.34 by Sebastian Bergmann and contributors.` -> 9.6.34.
/// Accepte les versions de développement (`11.0.0-dev`) et les lignes qui
/// continuent autrement (`PHPUnit 10.5.1 #StandWithUkraine`).
pub fn parse_phpunit_version(line: &str) -> Option<PhpUnitVersion> {
    let text = line.trim().strip_prefix("PHPUnit ")?;
    let text = text.split_whitespace().next()?;
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Some(PhpUnitVersion {
        major,
        minor,
        patch,
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "<?php\n").unwrap();
    }

    #[test]
    fn php_version_ignores_extension_banners() {
        let banner = "PHP 8.5.13 (cli) (built: Nov 18 2026) (ZTS Visual C++ 2022 x64)\n\
            Copyright (c) The PHP Group\n    with the ionCube PHP Loader v15.5.1\n";
        assert_eq!(parse_php_version(banner).as_deref(), Some("8.5.13"));
        assert_eq!(parse_php_version("rien"), None);
    }

    #[test]
    fn phpunit_version_lines() {
        let version =
            parse_phpunit_version("PHPUnit 9.6.34 by Sebastian Bergmann and contributors.")
                .unwrap();
        assert_eq!((version.major, version.minor, version.patch), (9, 6, 34));
        assert_eq!(version.text, "9.6.34");
        let dev = parse_phpunit_version("PHPUnit 11.0.0-dev by Sebastian Bergmann").unwrap();
        assert_eq!(
            (dev.major, dev.patch, dev.text.as_str()),
            (11, 0, "11.0.0-dev")
        );
        let flag = parse_phpunit_version("PHPUnit 10.5.1 #StandWithUkraine").unwrap();
        assert_eq!(flag.minor, 5);
        assert_eq!(parse_phpunit_version("Runtime: PHP 8.3.0"), None);
        assert_eq!(parse_phpunit_version("PHPUnit abc by x"), None);
    }

    #[test]
    fn phpunit_is_found_in_vendor_bin_then_in_the_package_then_as_phar() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let settings = PhpUnitSettings::default();

        let missing = locate_phpunit(&settings, root).unwrap_err();
        assert!(matches!(&missing, Problem::PhpUnitNotFound { searched } if searched.len() == 5));

        touch(&root.join("tools/phpunit.phar"));
        let found = locate_phpunit(&settings, root).unwrap();
        assert_eq!(found.kind, PhpUnitKind::Phar);

        touch(&root.join("vendor/phpunit/phpunit/phpunit"));
        let found = locate_phpunit(&settings, root).unwrap();
        assert_eq!(found.source, PhpUnitSource::VendorPackage);

        touch(&root.join("vendor/bin/phpunit"));
        let found = locate_phpunit(&settings, root).unwrap();
        assert_eq!(found.source, PhpUnitSource::VendorBin);
        assert_eq!(found.kind, PhpUnitKind::Script);
    }

    #[test]
    fn a_configured_phpunit_path_wins_and_a_bat_becomes_its_script() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("vendor/bin/phpunit"));
        touch(&root.join("autre/phpunit"));
        touch(&root.join("autre/phpunit.bat"));

        let settings = PhpUnitSettings {
            phpunit_path: Some(PathBuf::from("autre/phpunit.bat")),
            ..Default::default()
        };
        let found = locate_phpunit(&settings, root).unwrap();
        assert_eq!(found.path, root.join("autre/phpunit"));
        assert_eq!(found.source, PhpUnitSource::Settings);

        let settings = PhpUnitSettings {
            phpunit_path: Some(PathBuf::from("nulle/part")),
            ..Default::default()
        };
        assert_eq!(
            locate_phpunit(&settings, root).unwrap_err(),
            Problem::PhpUnitPathInvalid(PathBuf::from("nulle/part"))
        );
    }

    #[test]
    fn configuration_prefers_phpunit_xml_over_dist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let settings = PhpUnitSettings::default();
        assert!(matches!(
            locate_configuration(&settings, root),
            Err(Problem::NoConfiguration { .. })
        ));
        touch(&root.join("phpunit.xml.dist"));
        assert_eq!(
            locate_configuration(&settings, root).unwrap(),
            root.join("phpunit.xml.dist")
        );
        touch(&root.join("phpunit.xml"));
        assert_eq!(
            locate_configuration(&settings, root).unwrap(),
            root.join("phpunit.xml")
        );
        let settings = PhpUnitSettings {
            configuration: Some(PathBuf::from("absent.xml")),
            ..Default::default()
        };
        assert_eq!(
            locate_configuration(&settings, root).unwrap_err(),
            Problem::ConfigurationPathInvalid(PathBuf::from("absent.xml"))
        );
    }

    #[test]
    fn php_is_searched_in_settings_then_path_then_default() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bin = root.join("bin");
        let name = if cfg!(windows) { "php.exe" } else { "php" };
        touch(&bin.join(name));
        let path_var = std::env::join_paths([&bin]).unwrap();
        let absent_fallback = root.join("pas-de-php");
        let settings = PhpUnitSettings::default();

        let found = locate_php(&settings, root, Some(&path_var), &absent_fallback).unwrap();
        assert_eq!(found.source, PhpSource::Path);
        assert_eq!(found.path, bin.join(name));

        // Pas dans le PATH : le dernier recours.
        let fallback = root.join("secours").join(name);
        touch(&fallback);
        let found = locate_php(&settings, root, None, &fallback).unwrap();
        assert_eq!(found.source, PhpSource::DefaultInstall);

        // Rien nulle part.
        let error = locate_php(&settings, root, None, &absent_fallback).unwrap_err();
        assert!(matches!(error, Problem::PhpNotFound { .. }));

        // Le réglage l'emporte, et un réglage faux n'est pas contourné.
        let own = root.join("mon").join(name);
        touch(&own);
        let settings = PhpUnitSettings {
            php_path: Some(own.clone()),
            ..Default::default()
        };
        let found = locate_php(&settings, root, Some(&path_var), &fallback).unwrap();
        assert_eq!((found.path, found.source), (own, PhpSource::Settings));
        let settings = PhpUnitSettings {
            php_path: Some(root.join("faux").join(name)),
            ..Default::default()
        };
        assert!(matches!(
            locate_php(&settings, root, Some(&path_var), &fallback),
            Err(Problem::PhpPathInvalid(_))
        ));
        // Un nom nu est cherché dans le PATH.
        let settings = PhpUnitSettings {
            php_path: Some(PathBuf::from("php")),
            ..Default::default()
        };
        assert_eq!(
            locate_php(&settings, root, Some(&path_var), &fallback)
                .unwrap()
                .path,
            bin.join(name)
        );
    }

    #[test]
    fn plan_reports_the_first_problem() {
        let detection = Detection {
            root: PathBuf::from("."),
            php: Err(Problem::PhpPathInvalid(PathBuf::from("x"))),
            phpunit: Err(Problem::PhpUnitPathInvalid(PathBuf::from("y"))),
            configuration: Ok(PathBuf::from("phpunit.xml")),
        };
        assert!(!detection.is_ready());
        assert_eq!(detection.problems().len(), 2);
        assert_eq!(
            detection.plan().unwrap_err(),
            Problem::PhpPathInvalid(PathBuf::from("x"))
        );
    }
}
