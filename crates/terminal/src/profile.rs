//! Profils d'interpréteurs : cmd, Git Bash, PowerShell ; détection et réglages.

use std::path::{Path, PathBuf};

/// Chemin par défaut de Git Bash.
pub const DEFAULT_GIT_BASH: &str = r"C:\Program Files\Git\bin\bash.exe";

/// Sorte d'interpréteur. Clé de réglage : `"cmd"`, `"git-bash"`, `"powershell"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileKind {
    Cmd,
    GitBash,
    PowerShell,
}

impl ProfileKind {
    pub const ALL: [ProfileKind; 3] = [
        ProfileKind::Cmd,
        ProfileKind::GitBash,
        ProfileKind::PowerShell,
    ];

    /// Clé stable (réglages, catalogue de textes).
    pub fn key(self) -> &'static str {
        match self {
            ProfileKind::Cmd => "cmd",
            ProfileKind::GitBash => "git-bash",
            ProfileKind::PowerShell => "powershell",
        }
    }
}

/// Un interpréteur lançable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub kind: ProfileKind,
    /// Exécutable (chemin absolu).
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Variables posées en plus de l'environnement hérité de l'IDE.
    pub env: Vec<(String, String)>,
    /// L'exécutable existe (vérifié par [`detect_profiles`]). Un profil absent
    /// s'affiche grisé, avec `program` dans l'infobulle.
    pub available: bool,
}

/// Section `[terminal]` des réglages.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Terminal par défaut ; à défaut Git Bash s'il est installé, sinon cmd.
    pub default_profile: Option<ProfileKind>,
    /// Chemin de `bash.exe` (défaut [`DEFAULT_GIT_BASH`]).
    pub git_bash_path: Option<PathBuf>,
    /// Chemin de `cmd.exe` (défaut `%COMSPEC%`).
    pub cmd_path: Option<PathBuf>,
    /// Lignes d'historique (défaut 10 000).
    pub scrollback: Option<usize>,
}

impl TerminalSettings {
    pub fn scrollback(&self) -> usize {
        self.scrollback
            .unwrap_or(crate::emulator::DEFAULT_SCROLLBACK)
    }
}

/// Les profils détectés et celui par défaut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiles {
    /// Dans l'ordre de [`ProfileKind::ALL`], disponibles ou non.
    pub profiles: Vec<Profile>,
    /// Toujours un profil disponible si l'un l'est (cmd en dernier recours).
    pub default: ProfileKind,
}

impl Profiles {
    pub fn get(&self, kind: ProfileKind) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.kind == kind)
    }

    pub fn default_profile(&self) -> Option<&Profile> {
        self.get(self.default)
    }
}

fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

/// Le profil d'une sorte, sans accès disque (`available` à `true`).
pub fn profile(kind: ProfileKind, settings: &TerminalSettings) -> Profile {
    match kind {
        ProfileKind::Cmd => Profile {
            kind,
            program: settings
                .cmd_path
                .clone()
                .or_else(|| std::env::var_os("COMSPEC").map(PathBuf::from))
                .unwrap_or_else(|| system_root().join(r"System32\cmd.exe")),
            args: Vec::new(),
            env: Vec::new(),
            available: true,
        },
        ProfileKind::GitBash => Profile {
            kind,
            program: settings
                .git_bash_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(DEFAULT_GIT_BASH)),
            args: vec!["--login".into(), "-i".into()],
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("COLORTERM".into(), "truecolor".into()),
                // Coutume MSYS2 : un shell de connexion reste dans le dossier
                // de départ au lieu d'aller dans $HOME.
                ("CHERE_INVOKING".into(), "1".into()),
            ],
            available: true,
        },
        ProfileKind::PowerShell => Profile {
            kind,
            program: system_root().join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
            args: vec!["-NoLogo".into()],
            env: Vec::new(),
            available: true,
        },
    }
}

/// Détecte les interpréteurs installés. **Accès disque** (existence de trois
/// fichiers) : dans une tâche, ou par `Terminals::detect_profiles`.
pub fn detect_profiles(settings: &TerminalSettings) -> Profiles {
    detect_with(settings, |path| path.is_file())
}

fn detect_with(settings: &TerminalSettings, exists: impl Fn(&Path) -> bool) -> Profiles {
    let profiles: Vec<Profile> = ProfileKind::ALL
        .iter()
        .map(|&kind| {
            let mut profile = profile(kind, settings);
            profile.available = exists(&profile.program);
            profile
        })
        .collect();
    let available = |kind: ProfileKind| profiles.iter().any(|p| p.kind == kind && p.available);
    let default = settings
        .default_profile
        .filter(|&kind| available(kind))
        .or_else(|| available(ProfileKind::GitBash).then_some(ProfileKind::GitBash))
        .unwrap_or(ProfileKind::Cmd);
    Profiles { profiles, default }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_bash_is_default_when_installed() {
        let profiles = detect_with(&TerminalSettings::default(), |_| true);
        assert_eq!(profiles.default, ProfileKind::GitBash);
        let bash = profiles.get(ProfileKind::GitBash).unwrap();
        assert_eq!(bash.args, vec!["--login", "-i"]);
        assert!(bash
            .env
            .contains(&("TERM".to_string(), "xterm-256color".to_string())));
    }

    #[test]
    fn falls_back_to_cmd_and_greys_out_missing_bash() {
        let settings = TerminalSettings {
            git_bash_path: Some(PathBuf::from(r"Z:\nulle\part\bash.exe")),
            ..TerminalSettings::default()
        };
        let profiles = detect_profiles(&settings);
        assert_eq!(profiles.default, ProfileKind::Cmd);
        let bash = profiles.get(ProfileKind::GitBash).unwrap();
        assert!(!bash.available);
        assert_eq!(bash.program, PathBuf::from(r"Z:\nulle\part\bash.exe"));
    }

    #[test]
    fn setting_wins_when_available() {
        let settings = TerminalSettings {
            default_profile: Some(ProfileKind::Cmd),
            ..TerminalSettings::default()
        };
        assert_eq!(detect_with(&settings, |_| true).default, ProfileKind::Cmd);
        let settings = TerminalSettings {
            default_profile: Some(ProfileKind::PowerShell),
            ..TerminalSettings::default()
        };
        let profiles = detect_with(&settings, |p| !p.ends_with("powershell.exe"));
        assert_eq!(profiles.default, ProfileKind::GitBash);
    }

    #[test]
    fn settings_section_parses() {
        let settings: TerminalSettings = julie_core::Settings::from_layers(&[
            "[terminal]\ndefault_profile = \"cmd\"\ngit_bash_path = 'D:\\Git\\bin\\bash.exe'\nscrollback = 500\n",
        ])
        .unwrap()
        .section("terminal")
        .unwrap();
        assert_eq!(settings.default_profile, Some(ProfileKind::Cmd));
        assert_eq!(settings.scrollback(), 500);
    }

    #[test]
    fn real_machine_has_cmd() {
        let profiles = detect_profiles(&TerminalSettings::default());
        assert!(profiles.get(ProfileKind::Cmd).unwrap().available);
    }
}
