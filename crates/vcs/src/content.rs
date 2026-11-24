//! Contenu d'un fichier dans une version donnée (HEAD, index, étage de conflit,
//! commit, arbre de travail), pour le diff côte à côte et la fusion.
//!
//! Les versions tirées de Git passent par `git cat-file --filters` : on obtient
//! les octets **tels que Git les écrirait dans l'arbre de travail** (fins de ligne
//! `autocrlf`, filtres `.gitattributes`). Ainsi un fichier dont Git dit qu'il n'a
//! pas changé a les mêmes octets à gauche et à droite.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use julie_core::CancelToken;

use crate::error::{Result, VcsError};
use crate::git::{args, failure, Access, Git};
use crate::repo::Repository;

/// Au-delà, on ne charge pas le contenu (diff impossible, voir `git.md`).
pub const MAX_CONTENT_SIZE: u64 = 20 * 1024 * 1024;

/// Taille examinée pour reconnaître un fichier binaire (octet NUL).
const BINARY_PROBE: usize = 8 * 1024;

/// Étage de l'index pendant un conflit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Étage 1 : l'ancêtre commun.
    Base = 1,
    /// Étage 2 : « la nôtre » (pendant un rebase : la branche sur laquelle on rejoue).
    Ours = 2,
    /// Étage 3 : « la leur » (pendant un rebase : le commit rejoué).
    Theirs = 3,
}

/// D'où lire une version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Revision {
    /// Rien (volet vide : fichier ajouté, commit racine…).
    Absent,
    Head,
    /// Index, étage 0.
    Index,
    Stage(Stage),
    /// Un commit (hash, ou toute révision qui ne commence pas par `-`).
    Commit(String),
    /// Le fichier sur le disque.
    WorkTree,
}

/// Type d'une entrée Git.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileMode {
    Regular,
    Executable,
    Symlink,
}

/// Une version d'un fichier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Le fichier n'existe pas dans cette version.
    Missing,
    /// Contenu complet (pour un lien symbolique : sa cible).
    Blob { bytes: Vec<u8>, mode: FileMode },
    /// Plus de [`MAX_CONTENT_SIZE`] octets : non chargé.
    TooLarge { size: u64 },
    /// Sous-module : le commit enregistré.
    Submodule { commit: String },
    /// Un dossier sur le disque (sous-module ou dossier non suivi).
    Directory,
}

impl Content {
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Content::Blob { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    /// Prend les octets sans copie.
    pub fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            Content::Blob { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    pub fn exists(&self) -> bool {
        !matches!(self, Content::Missing)
    }

    /// Octet NUL dans les 8 premiers Kio (règle de `fusion.md`).
    pub fn is_binary(&self) -> bool {
        self.bytes().is_some_and(is_binary)
    }

    /// Taille en octets, quand elle est connue.
    pub fn size(&self) -> Option<u64> {
        match self {
            Content::Blob { bytes, .. } => Some(bytes.len() as u64),
            Content::TooLarge { size } => Some(*size),
            _ => None,
        }
    }
}

/// Octet NUL dans les 8 premiers Kio.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_PROBE)].contains(&0)
}

/// Une entrée d'arbre ou d'index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) mode: String,
    pub(crate) oid: String,
}

pub(crate) fn check_revision(revision: &str) -> Result<()> {
    if revision.is_empty() || revision.starts_with('-') || revision.contains(['\0', '\n']) {
        return Err(VcsError::InvalidRevision(revision.to_owned()));
    }
    Ok(())
}

/// Lit la version `revision` du fichier `path` (absolu, ou chemin Git).
/// **Bloquant.**
pub fn read_version(
    git: &Git,
    repo: &Repository,
    path: &Path,
    revision: &Revision,
    cancel: &CancelToken,
) -> Result<Content> {
    let repo_path = repo.to_repo_path(path)?;
    match revision {
        Revision::Absent => Ok(Content::Missing),
        Revision::WorkTree => read_work_tree(&repo.to_absolute(&repo_path)),
        Revision::Head => match tree_entry(git, repo, "HEAD", &repo_path, cancel) {
            Ok(entry) => entry_content(git, repo, &repo_path, entry, cancel),
            // Dépôt sans commit : HEAD n'existe pas, le volet est vide.
            Err(error @ VcsError::GitFailed { .. }) => {
                if has_head(git, repo, cancel)? {
                    Err(error)
                } else {
                    Ok(Content::Missing)
                }
            }
            Err(error) => Err(error),
        },
        Revision::Commit(commit) => {
            check_revision(commit)?;
            let entry = tree_entry(git, repo, commit, &repo_path, cancel)?;
            entry_content(git, repo, &repo_path, entry, cancel)
        }
        Revision::Index => {
            let entry = index_entries(git, repo, &repo_path, cancel)?
                .into_iter()
                .find(|(stage, _)| *stage == 0)
                .map(|(_, entry)| entry);
            entry_content(git, repo, &repo_path, entry, cancel)
        }
        Revision::Stage(stage) => {
            let entry = index_entries(git, repo, &repo_path, cancel)?
                .into_iter()
                .find(|(number, _)| *number == *stage as u8)
                .map(|(_, entry)| entry);
            entry_content(git, repo, &repo_path, entry, cancel)
        }
    }
}

pub(crate) fn has_head(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<bool> {
    let output = git.run(
        repo.work_tree(),
        &args(["rev-parse", "-q", "--verify", "HEAD^{commit}"]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(output.status.success())
}

/// Lit le fichier sur le disque.
pub(crate) fn read_work_tree(path: &Path) -> Result<Content> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Content::Missing),
        Err(source) => return Err(julie_core::Error::io(path, source).into()),
    };
    if meta.file_type().is_symlink() {
        let target =
            std::fs::read_link(path).map_err(|source| julie_core::Error::io(path, source))?;
        return Ok(Content::Blob {
            bytes: target.to_string_lossy().replace('\\', "/").into_bytes(),
            mode: FileMode::Symlink,
        });
    }
    if meta.is_dir() {
        return Ok(Content::Directory);
    }
    if meta.len() > MAX_CONTENT_SIZE {
        return Ok(Content::TooLarge { size: meta.len() });
    }
    let bytes = std::fs::read(path).map_err(|source| julie_core::Error::io(path, source))?;
    Ok(Content::Blob {
        bytes,
        mode: FileMode::Regular,
    })
}

/// L'entrée `repo_path` de l'arbre de `revision`, ou `None` si absente.
fn tree_entry(
    git: &Git,
    repo: &Repository,
    revision: &str,
    repo_path: &str,
    cancel: &CancelToken,
) -> Result<Option<Entry>> {
    let command: Vec<OsString> = args(["ls-tree", "-z", "--full-tree", revision, "--", repo_path]);
    let output = git.run_ok(repo.work_tree(), &command, Access::Read, None, cancel)?;
    for record in output.stdout.split(|&byte| byte == 0) {
        // `<mode> SP <type> SP <oid> TAB <chemin>`
        let Some(tab) = record.iter().position(|&byte| byte == b'\t') else {
            continue;
        };
        if &record[tab + 1..] != repo_path.as_bytes() {
            continue;
        }
        let head = String::from_utf8_lossy(&record[..tab]);
        let mut parts = head.split(' ');
        let (Some(mode), Some(_kind), Some(oid)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(VcsError::UnexpectedOutput(format!("ls-tree : {head:?}")));
        };
        return Ok(Some(Entry {
            mode: mode.to_owned(),
            oid: oid.to_owned(),
        }));
    }
    Ok(None)
}

/// Les entrées d'index de `repo_path`, avec leur étage (0 : normal ; 1, 2, 3 :
/// conflit).
pub(crate) fn index_entries(
    git: &Git,
    repo: &Repository,
    repo_path: &str,
    cancel: &CancelToken,
) -> Result<Vec<(u8, Entry)>> {
    let command = args(["ls-files", "-s", "-z", "--", repo_path]);
    let output = git.run_ok(repo.work_tree(), &command, Access::Read, None, cancel)?;
    let mut entries = Vec::new();
    for record in output.stdout.split(|&byte| byte == 0) {
        // `<mode> SP <oid> SP <étage> TAB <chemin>`
        let Some(tab) = record.iter().position(|&byte| byte == b'\t') else {
            continue;
        };
        if &record[tab + 1..] != repo_path.as_bytes() {
            continue;
        }
        let head = String::from_utf8_lossy(&record[..tab]);
        let parts: Vec<&str> = head.split(' ').collect();
        let [mode, oid, stage] = parts.as_slice() else {
            return Err(VcsError::UnexpectedOutput(format!("ls-files : {head:?}")));
        };
        let stage: u8 = stage
            .parse()
            .map_err(|_| VcsError::UnexpectedOutput(format!("ls-files : {head:?}")))?;
        entries.push((
            stage,
            Entry {
                mode: (*mode).to_owned(),
                oid: (*oid).to_owned(),
            },
        ));
    }
    Ok(entries)
}

/// Contenu d'une entrée Git.
pub(crate) fn entry_content(
    git: &Git,
    repo: &Repository,
    repo_path: &str,
    entry: Option<Entry>,
    cancel: &CancelToken,
) -> Result<Content> {
    let Some(entry) = entry else {
        return Ok(Content::Missing);
    };
    let mode = match entry.mode.as_str() {
        "160000" => return Ok(Content::Submodule { commit: entry.oid }),
        "120000" => FileMode::Symlink,
        "100755" => FileMode::Executable,
        _ => FileMode::Regular,
    };
    let size_output = git.run_ok(
        repo.work_tree(),
        &args(["cat-file", "-s", entry.oid.as_str()]),
        Access::Read,
        None,
        cancel,
    )?;
    let size: u64 = size_output
        .stdout_text()
        .trim()
        .parse()
        .map_err(|_| VcsError::UnexpectedOutput("cat-file -s".into()))?;
    if size > MAX_CONTENT_SIZE {
        return Ok(Content::TooLarge { size });
    }
    let command: Vec<OsString> = if mode == FileMode::Symlink {
        args(["cat-file", "blob", entry.oid.as_str()])
    } else {
        let mut path_arg = OsString::from("--path=");
        path_arg.push(repo_path);
        vec![
            "cat-file".into(),
            "--filters".into(),
            path_arg,
            entry.oid.as_str().into(),
        ]
    };
    let output = git.run(repo.work_tree(), &command, Access::Read, None, cancel)?;
    if !output.status.success() {
        return Err(failure(&command, &output));
    }
    Ok(Content::Blob {
        bytes: output.stdout,
        mode,
    })
}

/// Les deux volets d'un diff côte à côte.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiffRequest {
    /// Chemin absolu affiché (le nouveau chemin d'un renommage).
    pub path: PathBuf,
    /// Chemin du volet gauche quand il diffère (ancien chemin d'un renommage).
    pub left_path: Option<PathBuf>,
    pub left: Revision,
    pub right: Revision,
}

impl DiffRequest {
    /// Changement local : HEAD (ancien chemin si renommé) à gauche, disque à
    /// droite. Dans un dépôt sans commit, la gauche sera vide.
    pub fn local_change(file: &crate::status::FileStatus) -> Self {
        Self {
            path: file.path.clone(),
            left_path: file.original_path.clone(),
            left: Revision::Head,
            right: Revision::WorkTree,
        }
    }

    /// Fichier d'un commit : premier parent à gauche, le commit à droite.
    pub fn commit_file(commit: &str, file: &crate::history::CommitFile) -> Self {
        Self {
            path: file.path.clone(),
            left_path: file.original_path.clone(),
            left: file
                .parent
                .clone()
                .map(Revision::Commit)
                .unwrap_or(Revision::Absent),
            right: Revision::Commit(commit.to_owned()),
        }
    }
}

/// Résultat d'un [`DiffRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSides {
    pub request: DiffRequest,
    pub left: Content,
    pub right: Content,
}

/// Charge les deux volets. **Bloquant.**
pub fn load_diff(
    git: &Git,
    repo: &Repository,
    request: DiffRequest,
    cancel: &CancelToken,
) -> Result<DiffSides> {
    let left_path = request.left_path.as_ref().unwrap_or(&request.path);
    let left = read_version(git, repo, left_path, &request.left, cancel)?;
    let right = read_version(git, repo, &request.path, &request.right, cancel)?;
    Ok(DiffSides {
        request,
        left,
        right,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_detection_looks_at_the_first_8_kib() {
        assert!(is_binary(b"abc\0def"));
        assert!(!is_binary(b"texte normal\r\n"));
        let mut late = vec![b'a'; BINARY_PROBE];
        late.push(0);
        assert!(!is_binary(&late));
        assert!(Content::Blob {
            bytes: vec![0],
            mode: FileMode::Regular
        }
        .is_binary());
        assert!(!Content::Missing.is_binary());
    }

    #[test]
    fn revisions_starting_with_a_dash_are_refused() {
        assert!(check_revision("--output=x").is_err());
        assert!(check_revision("").is_err());
        assert!(check_revision("HEAD~1").is_ok());
    }
}
