//! Le sous-menu Git des fichiers (`menus-contextuels.md`) : ajouter à
//! l'index, retirer de l'index, annuler les modifications, renommer par
//! `git mv`, ajouter au `.gitignore`. L'historique d'un fichier passe par
//! [`crate::LogQuery::file_history`].

use std::path::{Path, PathBuf};

use julie_core::CancelToken;

use crate::content::has_head;
use crate::error::{Result, VcsError};
use crate::git::{args, failure, Access, Git};
use crate::repo::Repository;
use crate::stash::repo_paths;
use crate::status::{status, FileKind};

/// « Ajouter à Git » : `git add -- <chemins>` (un non suivi devient ajouté, un
/// modifié est indexé ; un dossier : ce qu'il contient). Chemin ignoré par
/// `.gitignore` : [`VcsError::IgnoredPaths`], rien n'est ajouté. **Bloquant.**
pub fn stage(git: &Git, repo: &Repository, paths: &[PathBuf], cancel: &CancelToken) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = args(["add", "--"]);
    command.extend(repo_paths(repo, paths)?);
    let output = git.run(repo.work_tree(), &command, Access::Write, None, cancel)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = output.stderr_text();
    if stderr.contains("ignored by one of your .gitignore files") {
        let ignored = stderr
            .lines()
            .skip_while(|line| !line.contains("ignored by one of your"))
            .skip(1)
            .take_while(|line| !line.starts_with("hint:") && !line.is_empty())
            .map(|line| repo.to_absolute(&crate::stash::unquote(line.trim())))
            .collect();
        return Err(VcsError::IgnoredPaths(ignored));
    }
    Err(failure(&command, &output))
}

/// « Retirer de l'index » : `git restore --staged -- <chemins>` ; dépôt sans
/// commit : `git rm --cached -r` (Git n'a pas de HEAD à restaurer). Le
/// fichier de travail n'est **jamais** touché. **Bloquant.**
pub fn unstage(
    git: &Git,
    repo: &Repository,
    paths: &[PathBuf],
    cancel: &CancelToken,
) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut command = if has_head(git, repo, cancel)? {
        args(["restore", "--staged", "--"])
    } else {
        // `--force` : seulement pour l'index (un fichier ajouté puis modifié).
        args([
            "rm",
            "--cached",
            "-r",
            "--force",
            "--quiet",
            "--ignore-unmatch",
            "--",
        ])
    };
    command.extend(repo_paths(repo, paths)?);
    git.run_ok(repo.work_tree(), &command, Access::Write, None, cancel)?;
    Ok(())
}

/// Ce qu'« Annuler les modifications » a fait.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscardOutcome {
    /// Ramenés à HEAD (index et travail) : modifiés, supprimés, ancien chemin
    /// d'un renommage.
    pub restored: Vec<PathBuf>,
    /// Fichiers ajoutés (absents de HEAD) retirés de l'index ; **ils restent
    /// sur le disque**. Si l'utilisatrice a coché « Supprimer aussi les
    /// fichiers ajoutés », c'est à l'appelant de les envoyer à la corbeille.
    pub unstaged_added: Vec<PathBuf>,
}

/// **Destructif** : « Annuler les modifications… » sur des fichiers ou des
/// dossiers (ce qu'ils contiennent). Modifiés et supprimés : `git restore
/// --source=HEAD --staged --worktree` ; ajoutés : seulement retirés de
/// l'index (jamais supprimés ici) ; renommés : l'ancien chemin revient, le
/// nouveau suit la règle de l'ajouté ; non suivis : jamais touchés. Dépôt sans
/// commit : seulement retirer de l'index. Fichier en conflit :
/// [`VcsError::Conflicted`], rien n'est fait. L'appelant a confirmé et pris sa
/// copie de sécurité. **Bloquant.**
pub fn discard_changes(
    git: &Git,
    repo: &Repository,
    paths: &[PathBuf],
    cancel: &CancelToken,
) -> Result<DiscardOutcome> {
    // `None` : la racine, donc tout.
    let selected: Vec<Option<String>> = paths
        .iter()
        .map(|path| {
            if repo.is_root(path) {
                Ok(None)
            } else {
                repo.to_repo_path(path).map(Some)
            }
        })
        .collect::<Result<_>>()?;
    let inside = |repo_path: &str| {
        selected.iter().any(|s| match s {
            None => true,
            Some(s) => repo_path == s || repo_path.starts_with(&format!("{s}/")),
        })
    };
    let current = status(git, repo, cancel)?;
    let unborn = current.head.is_unborn();
    let mut outcome = DiscardOutcome::default();
    for file in &current.files {
        let original_inside = file.original_repo_path.as_deref().is_some_and(inside);
        if !inside(&file.repo_path) && !original_inside {
            continue;
        }
        match file.kind {
            FileKind::Untracked => {}
            FileKind::Conflicted => return Err(VcsError::Conflicted(file.path.clone())),
            _ if unborn => outcome.unstaged_added.push(file.path.clone()),
            FileKind::Added => outcome.unstaged_added.push(file.path.clone()),
            FileKind::Renamed | FileKind::Copied => {
                outcome.unstaged_added.push(file.path.clone());
                if file.kind == FileKind::Renamed {
                    if let Some(original) = &file.original_path {
                        outcome.restored.push(original.clone());
                    }
                }
            }
            _ => outcome.restored.push(file.path.clone()),
        }
    }
    if !outcome.restored.is_empty() {
        let mut command = args(["restore", "--source=HEAD", "--staged", "--worktree", "--"]);
        command.extend(repo_paths(repo, &outcome.restored)?);
        git.run_ok(repo.work_tree(), &command, Access::Write, None, cancel)?;
    }
    unstage(git, repo, &outcome.unstaged_added, cancel)?;
    Ok(outcome)
}

/// Renomme ou déplace un fichier suivi par `git mv` (Git voit un renommage).
/// Un renommage qui ne change que la casse (`user.php` → `User.php`) passe par
/// un nom intermédiaire. Non suivi : [`VcsError::NotTracked`] (renommage
/// simple à faire par l'appelant) ; destination existante :
/// [`VcsError::DestinationExists`]. **Bloquant.**
pub fn move_path(
    git: &Git,
    repo: &Repository,
    from: &Path,
    to: &Path,
    cancel: &CancelToken,
) -> Result<()> {
    let from_repo = repo.to_repo_path(from)?;
    let to_repo = repo.to_repo_path(to)?;
    let tracked = git.read(
        repo.work_tree(),
        &args(["ls-files", "-z", "--", from_repo.as_str()]),
        cancel,
    )?;
    if tracked.is_empty() {
        return Err(VcsError::NotTracked(repo.to_absolute(&from_repo)));
    }
    let case_only = from_repo != to_repo && from_repo.to_lowercase() == to_repo.to_lowercase();
    if !case_only && repo.to_absolute(&to_repo).exists() {
        return Err(VcsError::DestinationExists(repo.to_absolute(&to_repo)));
    }
    // `git mv` ne crée pas les dossiers de la destination.
    if let Some(parent) = repo.to_absolute(&to_repo).parent() {
        std::fs::create_dir_all(parent).map_err(|source| julie_core::Error::io(parent, source))?;
    }
    let mv = |a: &str, b: &str| -> Result<()> {
        git.run_ok(
            repo.work_tree(),
            &args(["mv", "--", a, b]),
            Access::Write,
            None,
            cancel,
        )?;
        Ok(())
    };
    if case_only {
        let mut step = format!("{from_repo}.juliestorm-renommage");
        let mut n = 1;
        while repo.to_absolute(&step).exists() {
            n += 1;
            step = format!("{from_repo}.juliestorm-renommage-{n}");
        }
        mv(&from_repo, &step)?;
        if let Err(error) = mv(&step, &to_repo) {
            let _ = mv(&step, &from_repo);
            return Err(error);
        }
        Ok(())
    } else {
        mv(&from_repo, &to_repo)
    }
}

/// Ce qu'on ajoute au `.gitignore` de la racine.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IgnoreRule {
    /// Ce fichier (`/chemin`).
    File(PathBuf),
    /// Ce dossier (`/chemin/`).
    Folder(PathBuf),
    /// Tous les fichiers de cette extension (`*.log`), sans le point.
    Extension(String),
}

/// La ligne à ajouter, caractères spéciaux de `.gitignore` échappés.
/// Fonction pure.
pub fn ignore_line(repo: &Repository, rule: &IgnoreRule) -> Result<String> {
    Ok(match rule {
        IgnoreRule::File(path) => format!("/{}", escape_ignore(&repo.to_repo_path(path)?)),
        IgnoreRule::Folder(path) => format!("/{}/", escape_ignore(&repo.to_repo_path(path)?)),
        IgnoreRule::Extension(extension) => {
            let extension = extension.trim_start_matches('.');
            if extension.is_empty() || extension.contains('/') {
                return Err(VcsError::InvalidRequest(format!("extension {extension:?}")));
            }
            format!("*.{}", escape_ignore(extension))
        }
    })
}

fn escape_ignore(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '*' | '?' | '[' | '\\' | '!' | '#') {
            out.push('\\');
        }
        out.push(c);
    }
    // Espaces finals : ignorés par Git sauf échappés.
    let trimmed = out.trim_end_matches(' ');
    let spaces = out.len() - trimmed.len();
    let mut result = trimmed.to_owned();
    for _ in 0..spaces {
        result.push_str("\\ ");
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreAdded {
    /// Le `.gitignore` de la racine.
    pub file: PathBuf,
    pub line: String,
    /// La ligne y était déjà : rien n'a été écrit.
    pub already_present: bool,
}

/// Ajoute une ligne au `.gitignore` de la racine (créé s'il manque ; encodage
/// et fin de ligne du fichier respectés ; écriture atomique). Fichier déjà
/// suivi : [`VcsError::AlreadyTracked`] (l'ignorer ne le retirerait pas).
/// **Bloquant.**
pub fn add_to_gitignore(
    git: &Git,
    repo: &Repository,
    rule: &IgnoreRule,
    cancel: &CancelToken,
) -> Result<IgnoreAdded> {
    let line = ignore_line(repo, rule)?;
    if let IgnoreRule::File(path) = rule {
        let repo_path = repo.to_repo_path(path)?;
        let tracked = git.read(
            repo.work_tree(),
            &args(["ls-files", "-z", "--", repo_path.as_str()]),
            cancel,
        )?;
        if !tracked.is_empty() {
            return Err(VcsError::AlreadyTracked(repo.to_absolute(&repo_path)));
        }
    }
    let file = repo.work_tree().join(".gitignore");
    let (mut text, format) = match julie_core::file::read_text(&file) {
        Ok(existing) => (existing.text, existing.format),
        Err(julie_core::Error::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            (String::new(), julie_core::file::TextFormat::default())
        }
        Err(error) => return Err(error.into()),
    };
    if text
        .lines()
        .any(|existing| existing.trim_end_matches('\r') == line)
    {
        return Ok(IgnoreAdded {
            file,
            line,
            already_present: true,
        });
    }
    // Fichier mélangé : le texte est exact, on écrit la fin de ligne la plus
    // probable ; sinon `\n`, que l'écriture convertit.
    let newline =
        if format.line_ending == julie_core::file::LineEnding::Mixed && text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str(newline);
    }
    text.push_str(&line);
    text.push_str(newline);
    cancel.check()?;
    julie_core::file::write_text(&file, &text, format)?;
    Ok(IgnoreAdded {
        file,
        line,
        already_present: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignore_lines_are_anchored_and_escaped() {
        let repo = crate::repo::tests_support::repository(r"C:\d");
        assert_eq!(
            ignore_line(
                &repo,
                &IgnoreRule::File(PathBuf::from(r"C:\d\logs\a b.log"))
            )
            .unwrap(),
            "/logs/a b.log"
        );
        assert_eq!(
            ignore_line(&repo, &IgnoreRule::Folder(PathBuf::from(r"C:\d\cache"))).unwrap(),
            "/cache/"
        );
        assert_eq!(
            ignore_line(&repo, &IgnoreRule::File(PathBuf::from(r"C:\d\[x]*.txt"))).unwrap(),
            "/\\[x]\\*.txt"
        );
        assert_eq!(
            ignore_line(&repo, &IgnoreRule::Extension(".log".into())).unwrap(),
            "*.log"
        );
        assert_eq!(escape_ignore("fin "), "fin\\ ");
        assert!(ignore_line(&repo, &IgnoreRule::Extension(String::new())).is_err());
    }
}
