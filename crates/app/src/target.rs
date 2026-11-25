//! Ce sur quoi agit un menu contextuel (ou une action appelée au clavier) :
//! fichiers de l'arbre, des changements ou des onglets, commit du journal,
//! branche. Les zones remplissent un [`MenuRequest`] ; `app` le résout en menu.

use std::path::PathBuf;

use eframe::egui::Pos2;

use crate::branchui::BranchRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Tree,
    Changes,
    Tab,
}

impl Zone {
    pub fn key(self) -> &'static str {
        match self {
            Zone::Tree => "tree",
            Zone::Changes => "changes",
            Zone::Tab => "tab",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRef {
    pub id: String,
    pub short: String,
    pub subject: String,
    pub parents: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Files { zone: Zone, files: Vec<FileRef> },
    Commit(CommitRef),
    Branch(BranchRef),
}

impl Target {
    /// Pour l'état du mode scénario.
    pub fn describe(&self) -> String {
        match self {
            Target::Files { zone, files } => format!(
                "{}:{}",
                zone.key(),
                files
                    .iter()
                    .map(|f| f.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(";")
            ),
            Target::Commit(c) => format!("commit:{}", c.short),
            Target::Branch(b) => format!("branch:{}", b.name),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MenuRequest {
    pub target: Target,
    pub pos: Pos2,
    pub by_keyboard: bool,
}
