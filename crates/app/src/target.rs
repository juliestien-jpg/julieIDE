//! Ce sur quoi agit un menu contextuel (ou une action appelée au clavier) :
//! fichiers de l'arbre, des changements ou des onglets, commit du journal,
//! branche. Les zones remplissent un [`MenuRequest`] ; `app` le résout en menu.

use std::path::PathBuf;

use eframe::egui::Pos2;

use crate::branchui::BranchRef;
use julie_phpunit::{NodeId, NodeKind};

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

/// Un nœud de l'arbre des résultats de tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRef {
    pub id: NodeId,
    pub kind: NodeKind,
    pub label: String,
}

/// Une occurrence de la recherche dans les fichiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitRef {
    pub path: PathBuf,
    /// Ligne, base zéro.
    pub line: u32,
    /// Le texte de la ligne.
    pub text: String,
}

/// L'éditeur sous le clic : le fichier (s'il existe sur le disque) et l'état de la sélection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorRef {
    pub path: Option<PathBuf>,
    pub has_selection: bool,
    pub read_only: bool,
}

/// Le terminal (ou l'un de ses onglets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermRef {
    pub has_selection: bool,
    /// Numéro de l'onglet visé.
    pub tab: usize,
    /// Le clic était sur l'onglet (et non sur l'écran).
    pub on_tab: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Files { zone: Zone, files: Vec<FileRef> },
    Commit(CommitRef),
    Branch(BranchRef),
    Test(TestRef),
    Hit(HitRef),
    Editor(EditorRef),
    Terminal(TermRef),
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
            Target::Test(t) => format!("test:{}", t.label),
            Target::Hit(h) => format!("hit:{}:{}", h.path.display(), h.line + 1),
            Target::Editor(e) => format!(
                "editor:{}",
                e.path
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string())
            ),
            Target::Terminal(t) => format!("terminal:{}", t.tab + 1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MenuRequest {
    pub target: Target,
    pub pos: Pos2,
    pub by_keyboard: bool,
}
