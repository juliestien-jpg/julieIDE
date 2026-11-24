//! Réglages de la table `[phpunit]`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Section `[phpunit]` de `settings.toml`.
///
/// ```toml
/// [phpunit]
/// php_path = 'C:\php\php.exe'
/// phpunit_path = 'vendor\bin\phpunit'
/// configuration = 'phpunit.ci.xml'
/// timeout_seconds = 600
/// ```
///
/// Tout est facultatif. Les chemins relatifs se lisent depuis la racine du projet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PhpUnitSettings {
    /// Exécutable PHP. Un nom nu (`php`) est cherché dans le `PATH`.
    pub php_path: Option<PathBuf>,
    /// Script PHP (ou phar) de PHPUnit. Un `.bat` ou `.cmd` est remplacé par le
    /// script du même nom sans extension : on lance toujours `php <script>`.
    pub phpunit_path: Option<PathBuf>,
    /// Fichier de configuration à utiliser au lieu de `phpunit.xml(.dist)`.
    pub configuration: Option<PathBuf>,
    /// Durée maximale d'une exécution, en secondes ; absent ou 0 : pas de limite.
    pub timeout_seconds: Option<u64>,
}

impl PhpUnitSettings {
    /// Lit la section `[phpunit]` des réglages fusionnés.
    pub fn from_settings(settings: &julie_core::Settings) -> julie_core::Result<Self> {
        settings.section("phpunit")
    }

    /// Le délai maximal d'une exécution, s'il y en a un.
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout_seconds
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use julie_core::Settings;

    #[test]
    fn section_is_read_and_missing_section_is_default() {
        let settings = Settings::from_layers(&[
            "[phpunit]\nphp_path = 'C:\\php\\php.exe'\ntimeout_seconds = 30\n",
        ])
        .unwrap();
        let parsed = PhpUnitSettings::from_settings(&settings).unwrap();
        assert_eq!(parsed.php_path, Some(PathBuf::from(r"C:\php\php.exe")));
        assert_eq!(parsed.timeout(), Some(Duration::from_secs(30)));
        assert_eq!(parsed.phpunit_path, None);

        let empty = Settings::from_layers(&[]).unwrap();
        assert_eq!(
            PhpUnitSettings::from_settings(&empty).unwrap(),
            PhpUnitSettings::default()
        );
    }

    #[test]
    fn a_zero_timeout_means_no_limit() {
        let settings = PhpUnitSettings {
            timeout_seconds: Some(0),
            ..Default::default()
        };
        assert_eq!(settings.timeout(), None);
    }
}
