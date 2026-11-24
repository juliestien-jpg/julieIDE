//! Réglages : un fichier TOML par niveau (utilisateur, puis projet qui l'emporte),
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
//! default_shell = "git-bash"
//! ```
//!
//! Lecture seule pour l'instant : l'écriture viendra avec le dialogue de réglages.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::error::{Error, Result};
use crate::project::Project;

/// Réglages fusionnés. Immuable et peu coûteux à cloner : à passer par valeur aux
/// tâches de fond.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    merged: Table,
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
        let mut merged = Table::new();
        for path in [user, project].into_iter().flatten() {
            let text = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => return Err(Error::io(path, source)),
            };
            merge(&mut merged, parse(&text, &path.display().to_string())?);
        }
        Ok(Self { merged })
    }

    /// Construit des réglages à partir de textes TOML, du moins au plus prioritaire.
    pub fn from_layers(layers: &[&str]) -> Result<Self> {
        let mut merged = Table::new();
        for (index, text) in layers.iter().enumerate() {
            merge(&mut merged, parse(text, &format!("couche {index}"))?);
        }
        Ok(Self { merged })
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

fn parse(text: &str, origin: &str) -> Result<Table> {
    text.parse::<Table>().map_err(|error| Error::Settings {
        origin: origin.to_owned(),
        message: error.to_string(),
    })
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
}
