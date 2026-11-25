//! Le projet ouvert et ce qui l'entoure : services de fond (index, Git), messages
//! venus de fils d'aide (ouverture, sélecteur de dossier, liste d'un dossier), et
//! mémoire du dernier projet.
//!
//! Aucun accès disque sur le fil de l'interface : tout passe par un fil court qui
//! répond par un [`Msg`] et réveille l'interface.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use eframe::egui;
use julie_core::{Project, Settings};
use julie_index::IndexService;
use julie_vcs::{Repository, Status, Vcs};

/// Réponses des fils d'aide, lues une fois par image.
pub enum Msg {
    ProjectReady {
        project: Project,
        /// Échec de lecture des réglages : l'IDE continue avec les défauts.
        settings: Result<Settings, String>,
    },
    ProjectFailed {
        path: PathBuf,
        error: String,
    },
    /// Résultat du sélecteur de dossier (`None` : annulé).
    FolderPicked(Option<PathBuf>),
    LastProject(Option<PathBuf>),
    DirListed {
        dir: PathBuf,
        entries: Vec<TreeEntry>,
    },
}

#[derive(Clone)]
pub struct Channel {
    pub tx: Sender<Msg>,
    pub ctx: egui::Context,
}

impl Channel {
    pub fn send(&self, msg: Msg) {
        let _ = self.tx.send(msg);
        self.ctx.request_repaint();
    }
}

/// Une ligne de l'arbre du projet.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// Ignoré par Git (absent de l'index une fois celui-ci prêt).
    pub ignored: bool,
}

/// L'état Git connu.
pub enum GitInfo {
    /// Détection en cours.
    Unknown,
    NoRepository,
    Repository(Box<Status>),
}

pub struct ProjectState {
    pub project: Project,
    pub index: Arc<IndexService>,
    pub vcs: Vcs,
    pub repo: Option<Repository>,
    pub git: GitInfo,
    pub index_ready: bool,
}

impl ProjectState {
    /// Nom de la branche courante, ou `HEAD détachée`.
    pub fn branch_name(&self) -> Option<String> {
        match &self.git {
            GitInfo::Repository(status) => Some(match &status.head {
                julie_vcs::Head::Branch { name, .. } => name.clone(),
                julie_vcs::Head::Detached { commit } => {
                    format!("({})", commit.chars().take(7).collect::<String>())
                }
            }),
            _ => None,
        }
    }
}

/// Ouvre le dossier dans un fil : vérifie que c'est un dossier et lit les
/// réglages (accès disque).
pub fn spawn_open(path: PathBuf, channel: Channel) {
    std::thread::spawn(move || match Project::open(&path) {
        Ok(project) => {
            let settings = Settings::load(Some(&project)).map_err(|e| e.to_string());
            channel.send(Msg::ProjectReady { project, settings });
        }
        Err(error) => channel.send(Msg::ProjectFailed {
            path,
            error: error.to_string(),
        }),
    });
}

/// Sélecteur de dossier du système (bloquant : dans un fil).
pub fn spawn_pick_folder(start: Option<PathBuf>, channel: Channel) {
    std::thread::spawn(move || {
        let mut dialog = rfd::FileDialog::new().set_title(crate::texts::PICK_FOLDER_TITLE);
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        channel.send(Msg::FolderPicked(dialog.pick_folder()));
    });
}

/// `JULIESTORM_STATE_DIR` remplace `%APPDATA%\JulieStorm` (essais : on ne touche
/// pas à l'état de la personne qui utilise l'IDE).
pub fn state_dir_overridden() -> bool {
    std::env::var_os("JULIESTORM_STATE_DIR").is_some()
}

fn state_file() -> Option<PathBuf> {
    let dir = match std::env::var_os("JULIESTORM_STATE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("APPDATA")?).join("JulieStorm"),
    };
    Some(dir.join("last_project.txt"))
}

pub fn spawn_read_last(channel: Channel) {
    std::thread::spawn(move || {
        let last = state_file()
            .and_then(|file| std::fs::read_to_string(file).ok())
            .map(|text| PathBuf::from(text.trim()))
            .filter(|path| path.is_dir());
        channel.send(Msg::LastProject(last));
    });
}

pub fn spawn_write_last(root: PathBuf) {
    std::thread::spawn(move || {
        if let Some(file) = state_file() {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = julie_core::file::write_atomic(&file, root.to_string_lossy().as_bytes());
        }
    });
}

/// Liste un dossier de l'arbre dans un fil : dossiers d'abord, ordre alphabétique
/// sans casse, `.git` masqué. Les entrées absentes de l'index (une fois prêt)
/// sont marquées « ignorées ».
pub fn spawn_list_dir(dir: PathBuf, root: PathBuf, index: Arc<IndexService>, channel: Channel) {
    std::thread::spawn(move || {
        let entries = list_dir(&dir, &root, &index);
        channel.send(Msg::DirListed { dir, entries });
    });
}

fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

fn list_dir(dir: &Path, root: &Path, index: &IndexService) -> Vec<TreeEntry> {
    let relative = dir
        .strip_prefix(root)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let present: Option<HashSet<String>> = if index.is_ready() {
        let prefix = if relative.is_empty() {
            String::new()
        } else {
            format!("{relative}/")
        };
        Some(index.with_index(|files| {
            files
                .entries()
                .iter()
                .filter_map(|e| e.relative().strip_prefix(prefix.as_str()))
                .filter_map(|rest| rest.split('/').next())
                .map(str::to_owned)
                .collect()
        }))
    } else {
        None
    };
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<TreeEntry> = read
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" {
                return None;
            }
            let is_dir = entry.file_type().ok()?.is_dir();
            // Un dossier vide n'est pas dans l'index sans être « ignoré » pour autant.
            let ignored = present.as_ref().is_some_and(|set| !set.contains(&name))
                && !(is_dir && is_empty_dir(&entry.path()));
            Some(TreeEntry {
                path: entry.path(),
                name,
                is_dir,
                ignored,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}
