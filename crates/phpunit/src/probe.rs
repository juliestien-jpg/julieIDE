//! Sondes de validation des réglages : « ce chemin de PHP est-il bon ? », « ce script
//! est-il PHPUnit ? », « ce fichier de configuration est-il lisible ? ».
//!
//! Faites pour la fenêtre de réglages : à chaque frappe, l'interface lance une
//! tâche ([`spawn`] ou [`crate::PhpUnit::probe`]) et annule la précédente
//! (`Tasks::cancel`). Une sonde qui lance un processus a un délai de
//! [`PROBE_TIMEOUT`] (5 s) ; une sonde annulée ne publie rien. Les fonctions
//! `probe_*` sont **bloquantes** (accès disque, un processus) : jamais sur le fil de
//! l'interface.
//!
//! Aucune sonde n'écrit quoi que ce soit, et aucune ne lance de test : seulement
//! `php --version`, `php <script> --version` et la lecture du début d'un fichier.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use julie_core::process::{Command, Output};
use julie_core::{CancelToken, Domain, TaskHandle, Tasks};

use crate::detect::{
    locate_php, locate_phpunit, parse_php_version, parse_phpunit_version, PhpUnitVersion,
    DEFAULT_PHP_PATH,
};
use crate::error::Problem;
use crate::settings::PhpUnitSettings;

/// Délai d'une sonde qui lance un processus.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Combien d'octets d'un fichier de configuration on lit pour y chercher `<phpunit`.
const CONFIG_HEAD_BYTES: u64 = 64 * 1024;

/// Ce qu'on vérifie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeRequest {
    /// Un exécutable PHP (chemin, relatif à la racine, ou nom nu cherché dans le
    /// `PATH`).
    Php(PathBuf),
    /// Un script PHPUnit (`.phar`, `vendor/bin/phpunit`... ; un `.bat` est remplacé
    /// par son script). `php` : l'interpréteur à utiliser ; `None` : celui que la
    /// détection prendrait (`PATH`, puis le PHP de la machine de développement).
    PhpUnit {
        php: Option<PathBuf>,
        script: PathBuf,
    },
    /// Un fichier de configuration PHPUnit.
    Configuration(PathBuf),
}

/// Ce qu'une sonde réussie a trouvé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeSuccess {
    /// `path` : le fichier réellement lancé (nom nu résolu dans le `PATH`).
    Php {
        path: PathBuf,
        version: String,
    },
    PhpUnit {
        path: PathBuf,
        version: PhpUnitVersion,
    },
    Configuration {
        path: PathBuf,
    },
}

/// Ce qu'on reconnaît (pour « Ce n'est pas PHP : … »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeKind {
    Php,
    PhpUnit,
    Configuration,
}

/// Pourquoi une sonde échoue. Des données : l'interface choisit son texte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeFailure {
    /// « Ce chemin n'existe pas ».
    NotFound(PathBuf),
    /// C'est un dossier alors qu'on attend un fichier.
    IsDirectory(PathBuf),
    /// « Pas de réponse en 5 s ».
    TimedOut(Duration),
    /// Le programme répond, mais pas comme `kind` : « Ce n'est pas PHP : <première
    /// ligne de la sortie> ».
    NotThis { kind: ProbeKind, first_line: String },
    /// Le fichier existe mais ne se lance pas.
    NotRunnable { path: PathBuf, reason: String },
    /// Le fichier existe mais ne se lit pas.
    Unreadable { path: PathBuf, reason: String },
    /// Sonde de PHPUnit : PHP lui-même est inutilisable, la version de PHPUnit n'a
    /// pas pu être lue.
    PhpUnavailable,
}

pub type ProbeResult = Result<ProbeSuccess, ProbeFailure>;

/// Publié sur le bus (`Domain::PhpUnit`) à la fin d'une tâche [`spawn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeEvent {
    pub request: ProbeRequest,
    pub result: ProbeResult,
}

/// Lance une sonde en tâche de fond (`phpunit.probe`) ; publie un [`ProbeEvent`].
/// Annulée (`Tasks::cancel`) : rien n'est publié. `root` sert de dossier de travail
/// et de base aux chemins relatifs.
pub fn spawn(tasks: &Tasks, root: PathBuf, request: ProbeRequest) -> TaskHandle {
    tasks.spawn(Domain::PhpUnit, "phpunit.probe", move |ctx| {
        let result = probe(&root, &request, ctx.cancel_token())?;
        ctx.publish(ProbeEvent { request, result });
        Ok(())
    })
}

/// Fait la sonde demandée. `Err` seulement pour l'annulation.
pub fn probe(
    root: &Path,
    request: &ProbeRequest,
    cancel: &CancelToken,
) -> julie_core::Result<ProbeResult> {
    probe_with_timeout(root, request, cancel, PROBE_TIMEOUT)
}

pub fn probe_with_timeout(
    root: &Path,
    request: &ProbeRequest,
    cancel: &CancelToken,
    limit: Duration,
) -> julie_core::Result<ProbeResult> {
    cancel.check()?;
    match request {
        ProbeRequest::Php(path) => probe_php(root, path, cancel, limit),
        ProbeRequest::PhpUnit { php, script } => {
            probe_phpunit(root, php.as_deref(), script, cancel, limit)
        }
        ProbeRequest::Configuration(path) => Ok(probe_configuration(root, path)),
    }
}

/// `php --version` : le chemin existe, c'est un fichier, il répond « PHP x.y.z ».
pub fn probe_php(
    root: &Path,
    path: &Path,
    cancel: &CancelToken,
    limit: Duration,
) -> julie_core::Result<ProbeResult> {
    let resolved = match resolve_php(root, path) {
        Ok(resolved) => resolved,
        Err(failure) => return Ok(Err(failure)),
    };
    let command = Command::new(&resolved).arg("--version").current_dir(root);
    let output = match run_limited(&command, cancel, limit, &resolved)? {
        Ok(output) => output,
        Err(failure) => return Ok(Err(failure)),
    };
    let text = output.stdout_text();
    match parse_php_version(&text).filter(|_| output.status.success()) {
        Some(version) => Ok(Ok(ProbeSuccess::Php {
            path: resolved,
            version,
        })),
        None => Ok(Err(ProbeFailure::NotThis {
            kind: ProbeKind::Php,
            first_line: first_line(&text, &output.stderr_text()),
        })),
    }
}

/// `php <script> --version` : le script existe, c'est un fichier, il répond
/// « PHPUnit x.y.z ». `php` : voir [`ProbeRequest::PhpUnit`].
pub fn probe_phpunit(
    root: &Path,
    php: Option<&Path>,
    script: &Path,
    cancel: &CancelToken,
    limit: Duration,
) -> julie_core::Result<ProbeResult> {
    let settings = PhpUnitSettings {
        phpunit_path: Some(script.to_path_buf()),
        ..Default::default()
    };
    let resolved_script = match locate_phpunit(&settings, root) {
        Ok(info) => info.path,
        Err(_) => {
            let full = if script.is_absolute() {
                script.to_path_buf()
            } else {
                root.join(script)
            };
            return Ok(Err(if full.is_dir() {
                ProbeFailure::IsDirectory(script.to_path_buf())
            } else {
                ProbeFailure::NotFound(script.to_path_buf())
            }));
        }
    };
    let php_path = match php {
        Some(php) => resolve_php(root, php),
        None => locate_php(
            &PhpUnitSettings::default(),
            root,
            std::env::var_os("PATH").as_deref(),
            Path::new(DEFAULT_PHP_PATH),
        )
        .map(|info| info.path)
        .map_err(|_| ProbeFailure::PhpUnavailable),
    };
    let Ok(php_path) = php_path else {
        return Ok(Err(ProbeFailure::PhpUnavailable));
    };
    let command = Command::new(&php_path)
        .arg(&resolved_script)
        .arg("--version")
        .current_dir(root);
    let output = match run_limited(&command, cancel, limit, &php_path)? {
        Ok(output) => output,
        Err(failure) => return Ok(Err(failure)),
    };
    let text = output.stdout_text();
    match text.lines().find_map(parse_phpunit_version) {
        Some(version) => Ok(Ok(ProbeSuccess::PhpUnit {
            path: resolved_script,
            version,
        })),
        None => Ok(Err(ProbeFailure::NotThis {
            kind: ProbeKind::PhpUnit,
            first_line: first_line(&text, &output.stderr_text()),
        })),
    }
}

/// Le fichier existe, est un fichier, se lit et contient `<phpunit` dans ses
/// premiers 64 Kio. Pas de processus.
pub fn probe_configuration(root: &Path, path: &Path) -> ProbeResult {
    use std::io::Read;

    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let meta = std::fs::metadata(&full).map_err(|_| ProbeFailure::NotFound(path.to_path_buf()))?;
    if meta.is_dir() {
        return Err(ProbeFailure::IsDirectory(path.to_path_buf()));
    }
    let unreadable = |error: std::io::Error| ProbeFailure::Unreadable {
        path: full.clone(),
        reason: error.to_string(),
    };
    let file = std::fs::File::open(&full).map_err(unreadable)?;
    let mut head = Vec::new();
    file.take(CONFIG_HEAD_BYTES)
        .read_to_end(&mut head)
        .map_err(unreadable)?;
    let text = String::from_utf8_lossy(&head);
    if text.contains("<phpunit") {
        Ok(ProbeSuccess::Configuration { path: full })
    } else {
        Err(ProbeFailure::NotThis {
            kind: ProbeKind::Configuration,
            first_line: first_line(&text, ""),
        })
    }
}

/// Résout un chemin de PHP comme la détection (nom nu : `PATH`) et vérifie que
/// c'est un fichier.
fn resolve_php(root: &Path, path: &Path) -> Result<PathBuf, ProbeFailure> {
    let settings = PhpUnitSettings {
        php_path: Some(path.to_path_buf()),
        ..Default::default()
    };
    match locate_php(
        &settings,
        root,
        std::env::var_os("PATH").as_deref(),
        Path::new(DEFAULT_PHP_PATH),
    ) {
        Ok(info) => Ok(info.path),
        Err(Problem::PhpPathInvalid(configured)) => {
            let full = if configured.is_absolute() {
                configured.clone()
            } else {
                root.join(&configured)
            };
            Err(if full.is_dir() {
                ProbeFailure::IsDirectory(configured)
            } else {
                ProbeFailure::NotFound(configured)
            })
        }
        Err(_) => Err(ProbeFailure::NotFound(path.to_path_buf())),
    }
}

/// Lance `command` avec un délai ; l'annulation tue le processus.
fn run_limited(
    command: &Command,
    cancel: &CancelToken,
    limit: Duration,
    program: &Path,
) -> julie_core::Result<Result<Output, ProbeFailure>> {
    let inner = CancelToken::new();
    let done = AtomicBool::new(false);
    let timed_out = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let started = Instant::now();
            while !done.load(Ordering::SeqCst) {
                if cancel.is_cancelled() {
                    inner.cancel();
                } else if started.elapsed() >= limit {
                    timed_out.store(true, Ordering::SeqCst);
                    inner.cancel();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let result = command.run(&inner);
        done.store(true, Ordering::SeqCst);
        result
    });
    match result {
        Ok(output) => Ok(Ok(output)),
        Err(error) if error.is_cancelled() => {
            if timed_out.load(Ordering::SeqCst) {
                Ok(Err(ProbeFailure::TimedOut(limit)))
            } else {
                Err(julie_core::Error::Cancelled)
            }
        }
        Err(julie_core::Error::ProcessSpawn { source, .. }) => Ok(Err(ProbeFailure::NotRunnable {
            path: program.to_path_buf(),
            reason: source.to_string(),
        })),
        Err(error) => Err(error),
    }
}

/// La première ligne non vide de la sortie, à défaut de la sortie d'erreur.
fn first_line(stdout: &str, stderr: &str) -> String {
    [stdout, stderr]
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn real_php() -> Option<PathBuf> {
        let php = PathBuf::from(DEFAULT_PHP_PATH);
        if php.is_file() {
            Some(php)
        } else {
            eprintln!("essai sauté : PHP absent de cette machine");
            None
        }
    }

    fn ok<T>(value: julie_core::Result<T>) -> T {
        value.expect("pas annulé")
    }

    #[test]
    fn a_missing_or_directory_path_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("dossier")).unwrap();
        let cancel = CancelToken::new();
        let limit = Duration::from_secs(5);

        let missing = ok(probe_php(
            root,
            Path::new("nulle/part/php.exe"),
            &cancel,
            limit,
        ));
        assert_eq!(
            missing,
            Err(ProbeFailure::NotFound("nulle/part/php.exe".into()))
        );
        let directory = ok(probe_php(root, Path::new("dossier"), &cancel, limit));
        assert_eq!(directory, Err(ProbeFailure::IsDirectory("dossier".into())));
        let unit = ok(probe_phpunit(
            root,
            None,
            Path::new("dossier"),
            &cancel,
            limit,
        ));
        assert_eq!(unit, Err(ProbeFailure::IsDirectory("dossier".into())));
        let unit = ok(probe_phpunit(
            root,
            None,
            Path::new("absent.phar"),
            &cancel,
            limit,
        ));
        assert_eq!(unit, Err(ProbeFailure::NotFound("absent.phar".into())));
    }

    #[test]
    fn configuration_probe_checks_existence_kind_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("dossier")).unwrap();
        fs::write(
            root.join("bon.xml"),
            "<?xml version=\"1.0\"?>\n<phpunit colors=\"true\"/>",
        )
        .unwrap();
        fs::write(root.join("autre.xml"), "\n\n<config/>\n").unwrap();

        assert_eq!(
            probe_configuration(root, Path::new("bon.xml")),
            Ok(ProbeSuccess::Configuration {
                path: root.join("bon.xml")
            })
        );
        assert_eq!(
            probe_configuration(root, Path::new("absent.xml")),
            Err(ProbeFailure::NotFound("absent.xml".into()))
        );
        assert_eq!(
            probe_configuration(root, Path::new("dossier")),
            Err(ProbeFailure::IsDirectory("dossier".into()))
        );
        assert_eq!(
            probe_configuration(root, Path::new("autre.xml")),
            Err(ProbeFailure::NotThis {
                kind: ProbeKind::Configuration,
                first_line: "<config/>".into()
            })
        );
        // Un chemin absolu marche aussi.
        assert!(probe_configuration(Path::new("."), &root.join("bon.xml")).is_ok());
    }

    #[test]
    fn php_probe_reads_the_version() {
        let Some(php) = real_php() else { return };
        let dir = tempfile::tempdir().unwrap();
        let result = ok(probe_php(
            dir.path(),
            &php,
            &CancelToken::new(),
            PROBE_TIMEOUT,
        ));
        match result {
            Ok(ProbeSuccess::Php { path, version }) => {
                assert_eq!(path, php);
                assert!(
                    version.starts_with(|c: char| c.is_ascii_digit()),
                    "{version}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_program_that_is_not_php_says_what_it_printed() {
        if !cfg!(windows) {
            return;
        }
        let Some(system) = std::env::var_os("SystemRoot").map(PathBuf::from) else {
            return;
        };
        let hostname = system.join("System32").join("HOSTNAME.EXE");
        if !hostname.is_file() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let result = ok(probe_php(
            dir.path(),
            &hostname,
            &CancelToken::new(),
            PROBE_TIMEOUT,
        ));
        assert!(
            matches!(&result, Err(ProbeFailure::NotThis { kind: ProbeKind::Php, first_line }) if !first_line.is_empty()),
            "{result:?}"
        );
    }

    #[test]
    fn phpunit_probe_distinguishes_phpunit_from_any_php_script_and_times_out() {
        let Some(php) = real_php() else { return };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("faux.php"),
            "<?php echo \"Bonjour, je ne suis pas PHPUnit\\n\";\n",
        )
        .unwrap();
        fs::write(
            root.join("faux-phpunit.php"),
            "<?php echo \"PHPUnit 9.6.99 by Sebastian Bergmann and contributors.\\n\";\n",
        )
        .unwrap();
        fs::write(root.join("lent.php"), "<?php sleep(30);\n").unwrap();
        let cancel = CancelToken::new();

        let result = ok(probe_phpunit(
            root,
            Some(&php),
            Path::new("faux.php"),
            &cancel,
            PROBE_TIMEOUT,
        ));
        assert_eq!(
            result,
            Err(ProbeFailure::NotThis {
                kind: ProbeKind::PhpUnit,
                first_line: "Bonjour, je ne suis pas PHPUnit".into()
            })
        );
        let result = ok(probe_phpunit(
            root,
            Some(&php),
            Path::new("faux-phpunit.php"),
            &cancel,
            PROBE_TIMEOUT,
        ));
        match result {
            Ok(ProbeSuccess::PhpUnit { path, version }) => {
                assert_eq!(path, root.join("faux-phpunit.php"));
                assert_eq!(version.text, "9.6.99");
            }
            other => panic!("{other:?}"),
        }
        let started = Instant::now();
        let result = ok(probe_phpunit(
            root,
            Some(&php),
            Path::new("lent.php"),
            &cancel,
            Duration::from_millis(400),
        ));
        assert_eq!(
            result,
            Err(ProbeFailure::TimedOut(Duration::from_millis(400)))
        );
        assert!(started.elapsed() < Duration::from_secs(5));

        // PHP invalide : la version de PHPUnit n'est pas lue, la faute est à PHP.
        let result = ok(probe_phpunit(
            root,
            Some(Path::new("pas-de-php.exe")),
            Path::new("faux-phpunit.php"),
            &cancel,
            PROBE_TIMEOUT,
        ));
        assert_eq!(result, Err(ProbeFailure::PhpUnavailable));
    }

    #[test]
    fn a_cancelled_probe_publishes_nothing() {
        let Some(php) = real_php() else { return };
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("lent.php"), "<?php sleep(30);\n").unwrap();
        let bus = julie_core::Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let handle = spawn(
            &tasks,
            dir.path().to_path_buf(),
            ProbeRequest::PhpUnit {
                php: Some(php),
                script: "lent.php".into(),
            },
        );
        let id = handle.id();
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        tasks.cancel(id);
        handle.join();
        assert!(started.elapsed() < Duration::from_secs(3));
        while let Some(event) = bus.recv_timeout(Duration::from_millis(50)) {
            if let julie_core::Event::Domain(event) = event {
                assert!(event.get::<ProbeEvent>().is_none(), "sonde annulée publiée");
            }
        }
    }

    #[test]
    fn a_finished_probe_is_published_with_its_request() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("p.xml"), "<phpunit/>").unwrap();
        let bus = julie_core::Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let request = ProbeRequest::Configuration("p.xml".into());
        let handle = spawn(&tasks, dir.path().to_path_buf(), request.clone());
        let id = handle.id();
        handle.join();
        let mut seen = None;
        while let Some(event) = bus.recv_timeout(Duration::from_millis(100)) {
            if let julie_core::Event::Domain(event) = event {
                if event.task == Some(id) {
                    seen = event.get::<ProbeEvent>().cloned();
                }
            }
        }
        let seen = seen.expect("ProbeEvent publié");
        assert_eq!(seen.request, request);
        assert!(seen.result.is_ok());
    }
}
