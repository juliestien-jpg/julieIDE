//! Réglages de la table `[phpunit]`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Deserializer};

/// Section `[phpunit]` de `settings.toml`.
///
/// ```toml
/// [phpunit]
/// php_path = 'C:\php\php.exe'
/// phpunit_path = 'tools\phpunit\phpunit.phar'
/// configuration = 'phpunit.ci.xml'
/// extra_args = ["--stop-on-failure", "--group", "rapide"]
/// timeout_seconds = 600
///
/// [phpunit.env]
/// APP_ENV = "test"
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
    /// Options PHPUnit ajoutées à chaque exécution, avant celles de la demande.
    pub extra_args: Vec<String>,
    /// Variables d'environnement ajoutées à celles de l'IDE (`[phpunit.env]`) ;
    /// elles l'emportent sur elles. Les valeurs TOML numériques ou booléennes sont
    /// acceptées et écrites telles quelles.
    #[serde(deserialize_with = "lenient_env")]
    pub env: BTreeMap<String, String>,
    /// Durée maximale d'une exécution, en secondes ; absent ou 0 : pas de limite.
    pub timeout_seconds: Option<u64>,
}

/// Valeur de variable d'environnement : une chaîne, ou un nombre / booléen écrit
/// sans guillemets dans le fichier.
#[derive(Deserialize)]
#[serde(untagged)]
enum EnvValue {
    Text(String),
    Integer(i64),
    Float(f64),
    Bool(bool),
}

fn lenient_env<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = BTreeMap::<String, EnvValue>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|(key, value)| {
            let text = match value {
                EnvValue::Text(text) => text,
                EnvValue::Integer(number) => number.to_string(),
                EnvValue::Float(number) => number.to_string(),
                EnvValue::Bool(flag) => flag.to_string(),
            };
            (key, text)
        })
        .collect())
}

/// Options que l'IDE pose lui-même : les répéter dans `extra_args` casse la
/// commande ou la sortie analysée.
pub const RESERVED_ARGUMENTS: &[&str] = &[
    "--teamcity",
    "--configuration",
    "-c",
    "--printer",
    "--log-teamcity",
    "--no-configuration",
];

/// Un point douteux des réglages. N'empêche pas d'enregistrer : l'interface
/// l'affiche en avertissement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIssue {
    /// `extra_args` contient une option que l'IDE pose déjà.
    ReservedArgument(String),
    /// Nom de variable d'environnement vide ou contenant `=` ou un octet nul : la
    /// variable est ignorée au lancement.
    InvalidEnvName(String),
    /// Un argument vide : PHPUnit le prendrait pour un chemin.
    EmptyArgument,
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

    /// Les avertissements sur `extra_args` et `env` : arguments dans l'ordre de la
    /// liste, puis variables par ordre alphabétique.
    pub fn issues(&self) -> Vec<SettingsIssue> {
        let mut issues = Vec::new();
        for arg in &self.extra_args {
            if arg.is_empty() {
                issues.push(SettingsIssue::EmptyArgument);
            } else if is_reserved_argument(arg) {
                issues.push(SettingsIssue::ReservedArgument(arg.clone()));
            }
        }
        for name in self.env.keys() {
            if !is_valid_env_name(name) {
                issues.push(SettingsIssue::InvalidEnvName(name.clone()));
            }
        }
        issues
    }
}

/// `--teamcity`, `--configuration=x`, `-c`… (voir [`RESERVED_ARGUMENTS`]).
pub fn is_reserved_argument(arg: &str) -> bool {
    RESERVED_ARGUMENTS.iter().any(|reserved| {
        arg == *reserved
            || arg
                .strip_prefix(reserved)
                .is_some_and(|rest| rest.starts_with('='))
    })
}

/// Nom utilisable pour une variable d'environnement.
pub fn is_valid_env_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}

/// Découpe une ligne d'arguments saisie dans la fenêtre de réglages : aux espaces,
/// les guillemets doubles groupent (`--group "mon groupe"` : 2 arguments), `\"`
/// donne un guillemet littéral à l'intérieur d'un groupe. Un guillemet jamais
/// refermé groupe jusqu'à la fin de la ligne.
pub fn split_arguments(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            '\\' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// L'inverse de [`split_arguments`] : ce qu'on remet dans le champ de la fenêtre.
pub fn join_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.contains(char::is_whitespace) || arg.contains('"') {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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

    #[test]
    fn extra_args_and_env_are_read_from_the_file() {
        let settings = Settings::from_layers(&[
            "[phpunit]\nextra_args = ['--group', 'lent', '--stop-on-failure']\n\n[phpunit.env]\nAPP_ENV = 'test'\nNIVEAU = 3\nDEBUG = true\n",
        ])
        .unwrap();
        let parsed = PhpUnitSettings::from_settings(&settings).unwrap();
        assert_eq!(parsed.extra_args, ["--group", "lent", "--stop-on-failure"]);
        assert_eq!(parsed.env["APP_ENV"], "test");
        assert_eq!(parsed.env["NIVEAU"], "3");
        assert_eq!(parsed.env["DEBUG"], "true");
        assert!(parsed.issues().is_empty());
    }

    #[test]
    fn a_project_layer_replaces_the_whole_extra_args_list_and_merges_env_by_key() {
        let settings = Settings::from_layers(&[
            "[phpunit]\nextra_args = ['--a', '--b']\n[phpunit.env]\nX = '1'\nY = '2'\n",
            "[phpunit]\nextra_args = ['--c']\n[phpunit.env]\nY = '9'\n",
        ])
        .unwrap();
        let parsed = PhpUnitSettings::from_settings(&settings).unwrap();
        assert_eq!(parsed.extra_args, ["--c"]);
        assert_eq!(parsed.env["X"], "1");
        assert_eq!(parsed.env["Y"], "9");
    }

    #[test]
    fn issues_flag_reserved_options_and_bad_env_names() {
        let settings = PhpUnitSettings {
            extra_args: vec![
                "--teamcity".into(),
                "--configuration=autre.xml".into(),
                "-c".into(),
                "--stop-on-failure".into(),
                String::new(),
                "--configurationx".into(),
            ],
            env: [
                ("A=B".to_owned(), "x".to_owned()),
                (String::new(), "y".to_owned()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        assert_eq!(
            settings.issues(),
            [
                SettingsIssue::ReservedArgument("--teamcity".into()),
                SettingsIssue::ReservedArgument("--configuration=autre.xml".into()),
                SettingsIssue::ReservedArgument("-c".into()),
                SettingsIssue::EmptyArgument,
                SettingsIssue::InvalidEnvName(String::new()),
                SettingsIssue::InvalidEnvName("A=B".into()),
            ]
        );
    }

    #[test]
    fn argument_line_splits_on_spaces_and_groups_with_quotes() {
        assert_eq!(
            split_arguments(r#"--group "mon groupe" --stop-on-failure"#),
            ["--group", "mon groupe", "--stop-on-failure"]
        );
        assert_eq!(split_arguments("  a   b  "), ["a", "b"]);
        assert_eq!(split_arguments(r#"x "" y"#), ["x", "", "y"]);
        assert_eq!(split_arguments(r#""a \"b\" c""#), [r#"a "b" c"#]);
        assert_eq!(split_arguments(r#""non fermé ici"#), ["non fermé ici"]);
        assert!(split_arguments("   ").is_empty());
    }

    #[test]
    fn joining_then_splitting_gives_the_same_arguments() {
        let args: Vec<String> = ["--group", "mon groupe", "", r#"a"b"#, "--x=1"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(split_arguments(&join_arguments(&args)), args);
    }
}
