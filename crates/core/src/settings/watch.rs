//! Relecture des réglages quand leurs fichiers changent sur le disque.
//!
//! Un fil dédié regarde les deux fichiers (taille et date) toutes les
//! [`POLL_INTERVAL`] : deux appels système, rien de mesurable. Dès que le contenu
//! d'un fichier change — par la fenêtre de réglages, à la main dans l'IDE ou
//! depuis un autre programme — les réglages sont relus et publiés dans
//! [`Event::SettingsChanged`]. Un fichier devenu illisible ne fait pas perdre les
//! réglages : sa dernière version valide reste en vigueur et le problème est
//! signalé dans l'événement.

use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, Thread};
use std::time::{Duration, SystemTime};

use toml::Table;

use super::{parse_table, read_layer, Scope, Settings, SettingsFiles, SettingsProblem};
use crate::bus::Publisher;
use crate::error::Error;
use crate::event::Event;

/// Intervalle entre deux regards sur les fichiers (la spécification demande une
/// relecture en moins de deux secondes).
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Charge utile de [`Event::SettingsChanged`].
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsChanged {
    /// Les réglages en vigueur : pour chaque portée, la dernière version lisible
    /// de son fichier.
    pub settings: Settings,
    /// Sections (`"phpunit"`, `"vcs"`…) dont le contenu effectif a changé depuis
    /// l'événement précédent, triées. Au premier événement : toutes les sections
    /// présentes. Chaque service ne se reconfigure que si la sienne y figure.
    pub changed_sections: Vec<String>,
    /// Fichiers actuellement illisibles (au plus un par portée). Vide : tout va
    /// bien — y compris après la réparation d'un fichier signalé juste avant.
    pub problems: Vec<SettingsProblem>,
}

impl SettingsChanged {
    pub fn concerns(&self, section: &str) -> bool {
        self.changed_sections.iter().any(|name| name == section)
    }
}

/// Surveille les fichiers de réglages. Lâcher la valeur arrête la surveillance.
pub struct SettingsWatcher {
    shared: Arc<Shared>,
    thread: Thread,
}

struct Shared {
    stop: AtomicBool,
    force: AtomicBool,
    current: Mutex<Settings>,
}

impl SettingsWatcher {
    /// Lance la surveillance. Ne fait aucun accès disque dans le fil appelant : la
    /// première lecture a lieu dans le fil de surveillance et publie **toujours**
    /// un premier [`Event::SettingsChanged`], même sans aucun fichier.
    pub fn start(files: SettingsFiles, publisher: Publisher) -> Self {
        Self::start_with_interval(files, publisher, POLL_INTERVAL)
    }

    /// Comme [`SettingsWatcher::start`], avec un autre intervalle (tests).
    pub fn start_with_interval(
        files: SettingsFiles,
        publisher: Publisher,
        interval: Duration,
    ) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            force: AtomicBool::new(false),
            current: Mutex::new(Settings::default()),
        });
        let worker = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("julie-settings".to_owned())
            .spawn(move || {
                let mut state = WatchState::new(&files);
                while !worker.stop.load(Ordering::SeqCst) {
                    let force = worker.force.swap(false, Ordering::SeqCst);
                    if let Some(change) = state.poll(force) {
                        *worker
                            .current
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner) = change.settings.clone();
                        publisher.publish(Event::SettingsChanged(change));
                    }
                    thread::park_timeout(interval);
                }
            });
        let thread = match handle {
            Ok(handle) => handle.thread().clone(),
            // Plus de fil disponible : pas de surveillance, les réglages restent
            // ceux par défaut. `check_now` réveille alors le fil courant, sans effet.
            Err(_) => thread::current(),
        };
        Self { shared, thread }
    }

    /// Les derniers réglages publiés (valeurs par défaut avant le premier
    /// événement). Sans accès disque : utilisable dans le fil de l'interface.
    pub fn current(&self) -> Settings {
        self.shared
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Relit tout de suite, sans attendre le prochain regard : à appeler après
    /// [`SettingsFiles::apply`] pour que « Appliquer » prenne effet à l'instant.
    /// Ne bloque pas ; l'événement n'est publié que si quelque chose a changé.
    pub fn check_now(&self) {
        self.shared.force.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

impl Drop for SettingsWatcher {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// Ce que l'on sait d'un fichier sans le lire.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stamp {
    Missing,
    Present {
        len: u64,
        modified: Option<SystemTime>,
    },
    Unreadable,
}

struct Layer {
    scope: Scope,
    path: Option<PathBuf>,
    stamp: Option<Stamp>,
    /// Dernier texte lu (`None` : fichier absent ou illisible).
    text: Option<String>,
    /// Dernière version valide.
    table: Table,
    problem: Option<SettingsProblem>,
}

impl Layer {
    fn new(scope: Scope, path: Option<PathBuf>) -> Self {
        Self {
            scope,
            path,
            stamp: None,
            text: None,
            table: Table::new(),
            problem: None,
        }
    }

    fn refresh(&mut self, force: bool) {
        let Some(path) = &self.path else {
            return;
        };
        let stamp = match fs::metadata(path) {
            Ok(meta) => Stamp::Present {
                len: meta.len(),
                modified: meta.modified().ok(),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Stamp::Missing,
            Err(_) => Stamp::Unreadable,
        };
        if !force && self.stamp.as_ref() == Some(&stamp) {
            return;
        }
        let first = self.stamp.is_none();
        self.stamp = Some(stamp);

        let problem = |line, message| SettingsProblem {
            scope: self.scope,
            path: path.clone(),
            line,
            message,
        };
        match read_layer(path) {
            Ok(None) => {
                self.text = None;
                self.table = Table::new();
                self.problem = None;
            }
            Ok(Some(text)) => {
                if !first && self.text.as_deref() == Some(text.as_str()) {
                    return;
                }
                match parse_table(&text) {
                    Ok(table) => {
                        self.table = table;
                        self.problem = None;
                    }
                    Err((line, message)) => self.problem = Some(problem(Some(line), message)),
                }
                self.text = Some(text);
            }
            Err(error) => {
                let message = match error {
                    Error::Io { source, .. } => source.to_string(),
                    Error::Settings { message, .. } => message,
                    other => other.to_string(),
                };
                self.text = None;
                self.problem = Some(problem(None, message));
            }
        }
    }
}

/// L'état de la surveillance, séparé du fil pour se tester sans attendre.
struct WatchState {
    layers: [Layer; 2],
    published: Option<(Settings, Vec<SettingsProblem>)>,
}

impl WatchState {
    fn new(files: &SettingsFiles) -> Self {
        Self {
            layers: [
                Layer::new(Scope::User, files.path(Scope::User)),
                Layer::new(Scope::Project, files.path(Scope::Project)),
            ],
            published: None,
        }
    }

    /// Regarde les fichiers ; rend l'événement à publier s'il y a du nouveau.
    fn poll(&mut self, force: bool) -> Option<SettingsChanged> {
        for layer in &mut self.layers {
            layer.refresh(force);
        }
        let settings =
            Settings::from_tables(self.layers[0].table.clone(), self.layers[1].table.clone());
        let problems: Vec<SettingsProblem> = self
            .layers
            .iter()
            .filter_map(|layer| layer.problem.clone())
            .collect();

        let changed_sections = match &self.published {
            Some((before, before_problems)) => {
                if *before == settings && *before_problems == problems {
                    return None;
                }
                before.changed_sections(&settings)
            }
            None => Settings::default().changed_sections(&settings),
        };
        self.published = Some((settings.clone(), problems.clone()));
        Some(SettingsChanged {
            settings,
            changed_sections,
            problems,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::project::Project;
    use crate::settings::{Origin, SettingValue};

    fn fixture() -> (tempfile::TempDir, SettingsFiles) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projet d'été");
        fs::create_dir_all(&root).unwrap();
        let files = SettingsFiles::new(
            Some(
                dir.path()
                    .join("AppData")
                    .join("JulieStorm")
                    .join("settings.toml"),
            ),
            Some(Project::open(&root).unwrap()),
        );
        (dir, files)
    }

    #[test]
    fn the_first_poll_always_reports_then_only_real_changes() {
        let (_dir, files) = fixture();
        let mut state = WatchState::new(&files);

        let first = state.poll(false).expect("premier événement");
        assert_eq!(first.settings, Settings::default());
        assert!(first.changed_sections.is_empty() && first.problems.is_empty());
        assert!(state.poll(false).is_none());
        assert!(state.poll(true).is_none());

        files
            .set(
                Scope::User,
                &["app", "theme"],
                SettingValue::Text("clair".into()),
            )
            .unwrap();
        files
            .set(
                Scope::Project,
                &["phpunit", "php_path"],
                SettingValue::Path("php".into()),
            )
            .unwrap();
        let change = state.poll(false).expect("fichiers créés");
        assert_eq!(change.changed_sections, vec!["app", "phpunit"]);
        assert!(change.concerns("phpunit") && !change.concerns("vcs"));
        assert_eq!(change.settings.origin(&["app", "theme"]), Origin::User);
        assert!(state.poll(false).is_none());

        // Un commentaire ajouté à la main : le fichier change, pas les réglages.
        let path = files.path(Scope::Project).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("# rien de neuf\r\n");
        fs::write(&path, text).unwrap();
        assert!(state.poll(true).is_none());

        // Fichier supprimé : ses réglages disparaissent.
        fs::remove_file(&path).unwrap();
        let change = state.poll(false).expect("fichier supprimé");
        assert_eq!(change.changed_sections, vec!["phpunit"]);
        assert_eq!(change.settings.get(&["phpunit", "php_path"]), None);
    }

    #[test]
    fn a_broken_file_keeps_the_last_valid_settings_and_is_reported() {
        let (_dir, files) = fixture();
        let path = files.path(Scope::Project).unwrap();
        files
            .set(
                Scope::Project,
                &["phpunit", "php_path"],
                SettingValue::Path("php".into()),
            )
            .unwrap();
        let mut state = WatchState::new(&files);
        let valid = state.poll(false).unwrap().settings;

        fs::write(&path, "[phpunit\nphp_path = 'autre'\n").unwrap();
        let change = state.poll(true).expect("problème signalé");
        assert_eq!(change.settings, valid, "derniers réglages valides gardés");
        assert!(change.changed_sections.is_empty());
        assert_eq!(change.problems.len(), 1);
        assert_eq!(change.problems[0].scope, Scope::Project);
        assert_eq!(change.problems[0].path, path);
        assert_eq!(change.problems[0].line, Some(1));
        assert!(state.poll(true).is_none(), "signalé une seule fois");

        // Réparé : le problème disparaît et la nouvelle valeur s'applique.
        fs::write(&path, "[phpunit]\nphp_path = 'autre'\n").unwrap();
        let change = state.poll(true).expect("fichier réparé");
        assert!(change.problems.is_empty());
        assert_eq!(change.changed_sections, vec!["phpunit"]);
        assert_eq!(
            change
                .settings
                .get(&["phpunit", "php_path"])
                .and_then(|v| v.as_str()),
            Some("autre")
        );
    }

    #[test]
    fn a_file_broken_from_the_start_counts_as_empty() {
        let (_dir, files) = fixture();
        let path = files.path(Scope::User).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "theme = \n").unwrap();
        let mut state = WatchState::new(&files);
        let first = state.poll(false).unwrap();
        assert_eq!(first.settings, Settings::default());
        assert_eq!(first.problems.len(), 1);
        assert_eq!(first.problems[0].scope, Scope::User);
    }

    #[test]
    fn the_watcher_publishes_on_the_bus_when_a_file_changes_on_disk() {
        let (_dir, files) = fixture();
        let bus = Bus::new();
        let watcher = SettingsWatcher::start_with_interval(
            files.clone(),
            bus.publisher(),
            Duration::from_millis(20),
        );
        let next = |bus: &Bus| match bus.recv_timeout(Duration::from_secs(10)) {
            Some(Event::SettingsChanged(change)) => change,
            other => panic!("événement inattendu : {other:?}"),
        };

        assert_eq!(next(&bus).settings, Settings::default());

        // Écrit « à la main » : vu sans rien demander.
        let path = files.path(Scope::Project).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[vcs]\ngit_path = 'C:\\Git\\git.exe'\n").unwrap();
        let change = next(&bus);
        assert_eq!(change.changed_sections, vec!["vcs"]);
        assert_eq!(watcher.current(), change.settings);

        // Écrit par l'IDE puis `check_now` : pas besoin d'attendre le prochain regard.
        files.unset(Scope::Project, &["vcs", "git_path"]).unwrap();
        watcher.check_now();
        let change = next(&bus);
        assert_eq!(change.settings.get(&["vcs", "git_path"]), None);

        // Une fois lâché, plus rien n'est publié.
        drop(watcher);
        thread::sleep(Duration::from_millis(100));
        bus.drain();
        fs::write(&path, "[vcs]\ngit_path = 'git'\n").unwrap();
        assert!(bus.recv_timeout(Duration::from_millis(200)).is_none());
    }
}
