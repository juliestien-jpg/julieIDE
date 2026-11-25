//! Réglages : un fichier TOML par portée (utilisateur, puis projet qui l'emporte),
//! une table par crate.
//!
//! Le noyau ne connaît pas le contenu des sections : chaque crate déclare sa
//! structure (`#[derive(Deserialize, Default)]` avec `#[serde(default)]`) et la lit
//! avec [`Settings::section`].
//!
//! ```toml
//! [phpunit]
//! php_path = 'C:\php\php.exe'
//!
//! [terminal]
//! default_profile = "git-bash"
//! ```
//!
//! - [`Settings`] : la valeur fusionnée (défaut < utilisateur < projet, clé par
//!   clé) et, pour l'interface, l'origine de chaque valeur.
//! - [`SettingsFiles`] : où sont les fichiers, et leur **écriture** clé par clé,
//!   qui conserve commentaires, ordre et clés inconnues.
//! - [`SettingsWatcher`] : relit les fichiers quand ils changent sur le disque et
//!   publie [`crate::Event::SettingsChanged`].

mod files;
mod watch;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use toml::Table;

use crate::error::{Error, Result};
use crate::file::{self, Encoding};
use crate::project::Project;

pub use files::{SettingValue, SettingsEdit, SettingsFiles};
pub use toml::Value;
pub use watch::{SettingsChanged, SettingsWatcher};

/// Où un réglage est écrit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// `%APPDATA%\JulieStorm\settings.toml` : « Tous mes projets ».
    User,
    /// `<racine>\.juliestorm\settings.toml` : « Ce projet ».
    Project,
}

/// D'où vient la valeur effective d'une clé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Aucun fichier ne la règle : valeur par défaut ou détection automatique
    /// (c'est la crate propriétaire de la section qui sait laquelle).
    Default,
    User,
    Project,
}

/// Un fichier de réglages que l'on n'a pas pu comprendre. Tant qu'il est dans cet
/// état, l'IDE garde les derniers réglages valides et refuse d'y écrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsProblem {
    pub scope: Scope,
    pub path: PathBuf,
    /// Ligne de l'erreur, comptée à partir de **un** ; `None` si le fichier n'a
    /// pas pu être lu du tout.
    pub line: Option<u32>,
    /// Raison technique (en anglais pour une erreur de syntaxe TOML).
    pub message: String,
}

/// Réglages fusionnés. Immuable et peu coûteux à cloner : à passer par valeur aux
/// tâches de fond.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    merged: Table,
    user: Table,
    project: Table,
}

impl Settings {
    /// Charge les réglages de l'utilisateur puis ceux du projet. Un fichier absent
    /// compte pour vide ; un fichier illisible ou mal formé est une erreur
    /// ([`Error::Settings`]), pas un retour silencieux aux valeurs par défaut.
    /// Accès disque : tâche de fond.
    pub fn load(project: Option<&Project>) -> Result<Self> {
        let user = user_settings_path();
        let project = project.map(Project::settings_path);
        Self::load_files(user.as_deref(), project.as_deref())
    }

    /// Comme [`Settings::load`], avec des chemins explicites (tests).
    pub fn load_files(user: Option<&Path>, project: Option<&Path>) -> Result<Self> {
        let load = |path: Option<&Path>| -> Result<Table> {
            let Some(path) = path else {
                return Ok(Table::new());
            };
            match read_layer(path)? {
                None => Ok(Table::new()),
                Some(text) => parse(&text, &path.display().to_string()),
            }
        };
        Ok(Self::from_tables(load(user)?, load(project)?))
    }

    /// Construit des réglages à partir de textes TOML, du moins au plus prioritaire.
    /// Avec deux couches ou plus, la dernière tient lieu de portée projet et les
    /// précédentes, fusionnées, de portée utilisateur ; une couche seule est la
    /// portée utilisateur.
    pub fn from_layers(layers: &[&str]) -> Result<Self> {
        let mut tables = Vec::with_capacity(layers.len());
        for (index, text) in layers.iter().enumerate() {
            tables.push(parse(text, &format!("couche {index}"))?);
        }
        let project = if tables.len() >= 2 {
            tables.pop().unwrap_or_default()
        } else {
            Table::new()
        };
        let mut user = Table::new();
        for table in tables {
            merge(&mut user, table);
        }
        Ok(Self::from_tables(user, project))
    }

    pub(crate) fn from_tables(user: Table, project: Table) -> Self {
        let mut merged = user.clone();
        merge(&mut merged, project.clone());
        Self {
            merged,
            user,
            project,
        }
    }

    /// La section `name`, désérialisée dans le type de la crate appelante.
    /// Section absente : `T::default()`. Clé de mauvais type : [`Error::Settings`].
    pub fn section<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T> {
        match self.merged.get(name) {
            None => Ok(T::default()),
            Some(value) => {
                value
                    .clone()
                    .try_into()
                    .map_err(|error: toml::de::Error| Error::Settings {
                        origin: format!("section [{name}]"),
                        message: error.message().to_owned(),
                    })
            }
        }
    }

    /// La valeur effective à ce chemin (`["phpunit", "php_path"]`,
    /// `["phpunit", "env", "APP_ENV"]`), si un fichier la règle.
    pub fn get(&self, path: &[&str]) -> Option<&Value> {
        lookup(&self.merged, path)
    }

    /// La valeur écrite **à cette portée**, sans héritage : `None` si le fichier
    /// de cette portée ne règle pas la clé.
    pub fn get_in(&self, scope: Scope, path: &[&str]) -> Option<&Value> {
        lookup(self.layer(scope), path)
    }

    /// D'où vient la valeur effective de cette clé. Pour une table
    /// (`["phpunit", "env"]`), la portée la plus forte qui en règle au moins une
    /// clé.
    pub fn origin(&self, path: &[&str]) -> Origin {
        if lookup(&self.project, path).is_some() {
            Origin::Project
        } else if lookup(&self.user, path).is_some() {
            Origin::User
        } else {
            Origin::Default
        }
    }

    /// Ce dont hérite la portée `scope` : les réglages tels qu'ils seraient si son
    /// fichier était vide. Pour le projet, ceux de l'utilisateur ; pour
    /// l'utilisateur, rien (valeurs par défaut). Sert à montrer la valeur héritée
    /// d'un champ non réglé.
    pub fn inherited(&self, scope: Scope) -> Settings {
        match scope {
            Scope::User => Settings::default(),
            Scope::Project => Settings::from_tables(self.user.clone(), Table::new()),
        }
    }

    fn layer(&self, scope: Scope) -> &Table {
        match scope {
            Scope::User => &self.user,
            Scope::Project => &self.project,
        }
    }

    /// Noms des sections dont le contenu effectif diffère entre `self` et `other`.
    pub(crate) fn changed_sections(&self, other: &Settings) -> Vec<String> {
        let mut names: Vec<&String> = self.merged.keys().chain(other.merged.keys()).collect();
        names.sort();
        names.dedup();
        names
            .into_iter()
            .filter(|name| self.merged.get(*name) != other.merged.get(*name))
            .cloned()
            .collect()
    }
}

/// `%APPDATA%\JulieStorm\settings.toml` (ou `$XDG_CONFIG_HOME`, `~/.config` hors
/// Windows). `None` si aucun dossier de configuration n'est connu.
pub fn user_settings_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.filter(|base| !base.as_os_str().is_empty())
        .map(|base| base.join("JulieStorm").join("settings.toml"))
}

/// Texte d'un fichier de réglages, fins de ligne ramenées à `\n` et marque
/// d'ordre des octets retirée. `None` si le fichier n'existe pas.
fn read_layer(path: &Path) -> Result<Option<String>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::io(path, source)),
    };
    let decoded = file::decode(bytes);
    if decoded.format.encoding == Encoding::Windows1252 {
        return Err(Error::Settings {
            origin: path.display().to_string(),
            message: NOT_UTF8.to_owned(),
        });
    }
    Ok(Some(decoded.text))
}

const NOT_UTF8: &str = "le fichier n'est pas de l'UTF-8";

fn parse(text: &str, origin: &str) -> Result<Table> {
    text.parse::<Table>().map_err(|error| Error::Settings {
        origin: origin.to_owned(),
        message: error.to_string(),
    })
}

/// Lit et analyse le fichier d'une portée pour [`SettingsWatcher`] et
/// [`SettingsFiles::load_lenient`] : le problème est décrit avec sa ligne.
fn read_table(scope: Scope, path: &Path) -> std::result::Result<Table, SettingsProblem> {
    let problem = |line, message: String| SettingsProblem {
        scope,
        path: path.to_path_buf(),
        line,
        message,
    };
    let text = match read_layer(path) {
        Ok(None) => return Ok(Table::new()),
        Ok(Some(text)) => text,
        Err(Error::Io { source, .. }) => return Err(problem(None, source.to_string())),
        Err(Error::Settings { message, .. }) => return Err(problem(None, message)),
        Err(other) => return Err(problem(None, other.to_string())),
    };
    parse_table(&text).map_err(|(line, message)| problem(Some(line), message))
}

/// Analyse un texte TOML ; en cas d'erreur, la ligne (à partir de un) et la raison.
fn parse_table(text: &str) -> std::result::Result<Table, (u32, String)> {
    text.parse::<Table>().map_err(|error| {
        let start = error.span().map_or(0, |span| span.start.min(text.len()));
        (line_of(text, start), error.message().to_owned())
    })
}

/// Ligne (à partir de un) de l'octet `offset` dans `text`.
fn line_of(text: &str, offset: usize) -> u32 {
    let before = &text.as_bytes()[..offset.min(text.len())];
    let newlines = before.iter().filter(|&&byte| byte == b'\n').count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

fn lookup<'a>(table: &'a Table, path: &[&str]) -> Option<&'a Value> {
    let (first, rest) = path.split_first()?;
    let mut value = table.get(*first)?;
    for key in rest {
        value = value.as_table()?.get(*key)?;
    }
    Some(value)
}

/// Fusion en profondeur : les tables se combinent clé par clé, toute autre valeur
/// (tableaux compris) est remplacée en bloc.
fn merge(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(existing)), Value::Table(incoming)) => merge(existing, incoming),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(default)]
    struct FakePhpUnitSettings {
        php_path: Option<PathBuf>,
        arguments: Vec<String>,
        stop_on_failure: bool,
    }

    impl Default for FakePhpUnitSettings {
        fn default() -> Self {
            Self {
                php_path: None,
                arguments: vec!["--teamcity".into()],
                stop_on_failure: false,
            }
        }
    }

    #[test]
    fn a_missing_section_gives_the_defaults() {
        let settings = Settings::default();
        assert_eq!(
            settings.section::<FakePhpUnitSettings>("phpunit").unwrap(),
            FakePhpUnitSettings::default()
        );
    }

    #[test]
    fn project_overrides_user_key_by_key() {
        let settings = Settings::from_layers(&[
            "[phpunit]\nphp_path = 'C:\\php\\php.exe'\nstop_on_failure = true\n[terminal]\nshell = 'cmd'",
            "[phpunit]\nstop_on_failure = false\narguments = ['--testdox']",
        ])
        .unwrap();
        assert_eq!(
            settings.section::<FakePhpUnitSettings>("phpunit").unwrap(),
            FakePhpUnitSettings {
                php_path: Some(PathBuf::from(r"C:\php\php.exe")),
                arguments: vec!["--testdox".into()],
                stop_on_failure: false,
            }
        );
    }

    #[test]
    fn bad_syntax_and_bad_types_are_errors() {
        assert!(matches!(
            Settings::from_layers(&["[phpunit\n"]),
            Err(Error::Settings { .. })
        ));
        let settings = Settings::from_layers(&["[phpunit]\nstop_on_failure = 'oui'"]).unwrap();
        assert!(matches!(
            settings.section::<FakePhpUnitSettings>("phpunit"),
            Err(Error::Settings { .. })
        ));
    }

    #[test]
    fn files_are_loaded_and_missing_ones_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        fs::write(&user, "[phpunit]\nstop_on_failure = true\n").unwrap();
        let settings =
            Settings::load_files(Some(&user), Some(&dir.path().join("absent.toml"))).unwrap();
        assert!(
            settings
                .section::<FakePhpUnitSettings>("phpunit")
                .unwrap()
                .stop_on_failure
        );

        fs::write(&user, "ceci n'est pas du TOML").unwrap();
        assert!(Settings::load_files(Some(&user), None).is_err());
    }

    #[test]
    fn a_file_with_bom_and_crlf_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user.toml");
        fs::write(
            &user,
            b"\xEF\xBB\xBF# r\xC3\xA9glages\r\n[phpunit]\r\nstop_on_failure = true\r\n",
        )
        .unwrap();
        let settings = Settings::load_files(Some(&user), None).unwrap();
        assert_eq!(
            settings.get(&["phpunit", "stop_on_failure"]),
            Some(&Value::Boolean(true))
        );

        // Pas de l'UTF-8 : refusé, pas deviné.
        fs::write(&user, b"[phpunit]\nnote = '\xE9t\xE9'\n").unwrap();
        assert!(matches!(
            Settings::load_files(Some(&user), None),
            Err(Error::Settings { .. })
        ));
    }

    #[test]
    fn origin_and_inherited_values_are_known_key_by_key() {
        let settings = Settings::from_layers(&[
            "[phpunit]\nphp_path = 'C:\\php\\php.exe'\nphpunit_path = 'a.phar'\n[phpunit.env]\nA = '1'\nB = '2'\n",
            "[phpunit]\nphpunit_path = 'b.phar'\n[phpunit.env]\nB = 'deux'\n",
        ])
        .unwrap();

        assert_eq!(settings.origin(&["phpunit", "php_path"]), Origin::User);
        assert_eq!(
            settings.origin(&["phpunit", "phpunit_path"]),
            Origin::Project
        );
        assert_eq!(
            settings.origin(&["phpunit", "configuration"]),
            Origin::Default
        );
        assert_eq!(settings.origin(&["phpunit", "env", "A"]), Origin::User);
        assert_eq!(settings.origin(&["phpunit", "env", "B"]), Origin::Project);
        assert_eq!(settings.origin(&["vcs", "git_path"]), Origin::Default);

        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        assert_eq!(
            text(settings.get(&["phpunit", "phpunit_path"])).as_deref(),
            Some("b.phar")
        );
        assert_eq!(
            text(settings.get_in(Scope::User, &["phpunit", "phpunit_path"])).as_deref(),
            Some("a.phar")
        );
        assert_eq!(
            settings.get_in(Scope::Project, &["phpunit", "php_path"]),
            None
        );
        assert_eq!(
            text(settings.get(&["phpunit", "env", "A"])).as_deref(),
            Some("1")
        );

        // Ce dont hérite le projet : la valeur de l'utilisateur.
        let inherited = settings.inherited(Scope::Project);
        assert_eq!(
            text(inherited.get(&["phpunit", "phpunit_path"])).as_deref(),
            Some("a.phar")
        );
        assert_eq!(settings.inherited(Scope::User), Settings::default());
    }

    #[test]
    fn changed_sections_compare_effective_content() {
        let before = Settings::from_layers(&["[phpunit]\na = 1\n[vcs]\nb = 2\n"]).unwrap();
        let after =
            Settings::from_layers(&["[phpunit]\na = 1\n[vcs]\nb = 3\n[app]\ntheme = 'clair'\n"])
                .unwrap();
        assert_eq!(before.changed_sections(&after), vec!["app", "vcs"]);
        assert!(before.changed_sections(&before).is_empty());
    }

    #[test]
    fn a_parse_error_knows_its_line() {
        let (line, _) = parse_table("# un\n[phpunit]\nphp_path = 'x'\n[vcs\n").unwrap_err();
        assert_eq!(line, 4);
    }
}
