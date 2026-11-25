//! Détection du dépôt, conversion des chemins, opération en cours.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use julie_core::CancelToken;

use crate::error::{Result, VcsError};
use crate::git::{args, failure, Access, Git};

/// Un dépôt Git ouvert : son arbre de travail et son dossier `.git`.
/// Peu coûteux à cloner ; à passer par valeur aux tâches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repository {
    work_tree: PathBuf,
    git_dir: PathBuf,
}

impl Repository {
    /// Racine de l'arbre de travail, écrite **comme le chemin donné à
    /// [`open_repository`]** (pas de canonisation : `Project::relative` marche).
    pub fn work_tree(&self) -> &Path {
        &self.work_tree
    }

    /// Dossier Git (`.git`, ou celui d'un arbre de travail lié), absolu.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// Chemin Git (relatif à la racine, séparé par `/`) d'un chemin absolu de
    /// l'arbre de travail, ou d'un chemin déjà relatif à la racine. Sous Windows,
    /// la casse du préfixe est ignorée.
    pub fn to_repo_path(&self, path: &Path) -> Result<String> {
        let relative = if path.is_absolute() {
            strip_prefix(path, &self.work_tree)
                .ok_or_else(|| VcsError::OutsideRepository(path.to_path_buf()))?
        } else {
            path.to_path_buf()
        };
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(
                    part.to_str()
                        .ok_or_else(|| VcsError::OutsideRepository(path.to_path_buf()))?
                        .to_owned(),
                ),
                Component::CurDir => {}
                _ => return Err(VcsError::OutsideRepository(path.to_path_buf())),
            }
        }
        if parts.is_empty() {
            return Err(VcsError::OutsideRepository(path.to_path_buf()));
        }
        Ok(parts.join("/"))
    }

    /// Le chemin désigne-t-il la racine de l'arbre de travail elle-même ?
    pub fn is_root(&self, path: &Path) -> bool {
        let rest = if path.is_absolute() {
            match strip_prefix(path, &self.work_tree) {
                Some(rest) => rest,
                None => return false,
            }
        } else {
            path.to_path_buf()
        };
        rest.components().all(|c| matches!(c, Component::CurDir))
    }

    /// Chemin absolu d'un chemin Git (`dossier/fichier.php`).
    pub fn to_absolute(&self, repo_path: &str) -> PathBuf {
        let mut path = self.work_tree.clone();
        for part in repo_path.split('/').filter(|part| !part.is_empty()) {
            path.push(part);
        }
        path
    }

    /// Le dépôt est-il en train de… (fusion, rebase…) ? Lit le dossier Git.
    /// Accès disque : tâche de fond.
    pub fn operation(&self) -> Option<Operation> {
        detect_operation(&self.git_dir)
    }
}

/// Retire `prefix` de `path`, composant par composant, sans tenir compte de la
/// casse sous Windows.
fn strip_prefix(path: &Path, prefix: &Path) -> Option<PathBuf> {
    if let Ok(rest) = path.strip_prefix(prefix) {
        return Some(rest.to_path_buf());
    }
    if !cfg!(windows) {
        return None;
    }
    let mut path_parts = path.components();
    for expected in prefix.components() {
        let actual = path_parts.next()?;
        let same = actual
            .as_os_str()
            .to_string_lossy()
            .to_lowercase()
            .replace('/', "\\")
            == expected
                .as_os_str()
                .to_string_lossy()
                .to_lowercase()
                .replace('/', "\\");
        if !same {
            return None;
        }
    }
    Some(path_parts.as_path().to_path_buf())
}

/// Ouvre le dépôt qui contient `dir` (le dossier lui-même ou un parent).
/// **Bloquant** (lance `git rev-parse`).
pub fn open_repository(git: &Git, dir: &Path, cancel: &CancelToken) -> Result<Repository> {
    let dir = std::path::absolute(dir).map_err(|source| julie_core::Error::io(dir, source))?;
    if !dir.is_dir() {
        return Err(VcsError::NotARepository(dir));
    }
    let command: Vec<OsString> = args([
        "rev-parse",
        "--is-inside-work-tree",
        "--absolute-git-dir",
        "--show-cdup",
    ]);
    let output = git.run(&dir, &command, Access::Read, None, cancel)?;
    if !output.status.success() {
        let stderr = output.stderr_text();
        if stderr.contains("not a git repository") || stderr.contains("must be run in a work tree")
        {
            return Err(VcsError::NotARepository(dir));
        }
        return Err(failure(&command, &output));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| VcsError::UnexpectedOutput("rev-parse : pas de l'UTF-8".into()))?;
    let mut lines = text.lines().map(str::trim);
    // Dans `.git` (ou un dépôt nu) : pas d'arbre de travail.
    if lines.next() != Some("true") {
        return Err(VcsError::NotARepository(dir));
    }
    let git_dir = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| VcsError::UnexpectedOutput(format!("rev-parse : {text:?}")))?;
    let cdup = lines.next().unwrap_or_default();
    let work_tree = normalize(&dir.join(cdup));
    Ok(Repository {
        work_tree,
        git_dir: PathBuf::from(git_dir),
    })
}

/// Résout lexicalement `.` et `..`, sans accès disque.
fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Opération Git interrompue (en général par un conflit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationKind {
    Merge,
    /// Rebase (`rebase-merge` ou `rebase-apply`, ce dernier couvrant aussi `git am`).
    Rebase,
    CherryPick,
    Revert,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub kind: OperationKind,
    /// Contenu de `.git/MERGE_MSG` s'il existe (message proposé pour conclure).
    pub message: Option<String>,
    /// Le commit « leur » : `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD` ou,
    /// pendant un rebase, `REBASE_HEAD` (le commit rejoué).
    pub their_commit: Option<String>,
    /// Pendant un rebase : la branche rejouée (`refs/heads/x` → `x`).
    pub rebase_branch: Option<String>,
    /// Pendant un rebase : commit en cours et nombre total (« commit 2 sur 3 »).
    pub step: Option<(u32, u32)>,
    /// Pendant un rebase : le commit sur lequel on rejoue.
    pub onto: Option<String>,
}

/// Branche courante lue dans le fichier `HEAD` du dossier Git, sans lancer Git :
/// `Some(nom)` sur une branche (née ou à naître), `None` si HEAD est détachée ou
/// illisible.
pub(crate) fn current_branch_name(git_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    text.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_owned)
}

pub(crate) fn detect_operation(git_dir: &Path) -> Option<Operation> {
    let read = |name: &str| {
        std::fs::read_to_string(git_dir.join(name))
            .ok()
            .map(|text| text.trim_end().to_owned())
            .filter(|text| !text.is_empty())
    };
    let first_line =
        |name: &str| read(name).and_then(|text| text.lines().next().map(str::to_owned));
    let message = read("MERGE_MSG");

    for dir in ["rebase-merge", "rebase-apply"] {
        if git_dir.join(dir).is_dir() {
            let branch = first_line(&format!("{dir}/head-name")).map(|name| {
                name.strip_prefix("refs/heads/")
                    .map(str::to_owned)
                    .unwrap_or(name)
            });
            let number = |name: &str| {
                first_line(&format!("{dir}/{name}"))
                    .and_then(|text| text.trim().parse::<u32>().ok())
            };
            let step = match dir {
                "rebase-merge" => number("msgnum").zip(number("end")),
                _ => number("next").zip(number("last")),
            };
            return Some(Operation {
                kind: OperationKind::Rebase,
                message,
                their_commit: first_line("REBASE_HEAD"),
                rebase_branch: branch,
                step,
                onto: first_line(&format!("{dir}/onto")),
            });
        }
    }
    let simple = [
        ("MERGE_HEAD", OperationKind::Merge),
        ("CHERRY_PICK_HEAD", OperationKind::CherryPick),
        ("REVERT_HEAD", OperationKind::Revert),
    ];
    simple.into_iter().find_map(|(file, kind)| {
        first_line(file).map(|commit| Operation {
            kind,
            message: message.clone(),
            their_commit: Some(commit),
            rebase_branch: None,
            step: None,
            onto: None,
        })
    })
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// Un dépôt fictif, sans disque, pour les tests d'analyse.
    pub(crate) fn repository(root: &str) -> Repository {
        Repository {
            work_tree: PathBuf::from(root),
            git_dir: PathBuf::from(root).join(".git"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(root: &str) -> Repository {
        tests_support::repository(root)
    }

    #[test]
    fn repo_paths_round_trip() {
        let repo = repo(r"C:\Dépôts\mon projet");
        let absolute = repo.to_absolute("dossier é/fichier un.php");
        assert_eq!(
            absolute,
            PathBuf::from(r"C:\Dépôts\mon projet\dossier é\fichier un.php")
        );
        assert_eq!(
            repo.to_repo_path(&absolute).unwrap(),
            "dossier é/fichier un.php"
        );
        assert_eq!(repo.to_repo_path(Path::new("a/b.txt")).unwrap(), "a/b.txt");
    }

    #[cfg(windows)]
    #[test]
    fn prefix_case_is_ignored_on_windows() {
        let repo = repo(r"C:\Dépôts\Projet");
        assert_eq!(
            repo.to_repo_path(Path::new(r"c:\DÉPÔTS\projet\Src\A.php"))
                .unwrap(),
            "Src/A.php"
        );
    }

    #[test]
    fn paths_outside_are_refused() {
        let repo = repo(r"C:\depot");
        assert!(repo.to_repo_path(Path::new(r"C:\ailleurs\x")).is_err());
        assert!(repo.to_repo_path(Path::new(r"C:\depot")).is_err());
        assert!(repo.to_repo_path(Path::new("../x")).is_err());
    }

    #[test]
    fn normalize_resolves_parents() {
        assert_eq!(
            normalize(Path::new(r"C:\a\b\c\..\..\")),
            PathBuf::from(r"C:\a")
        );
    }
}
