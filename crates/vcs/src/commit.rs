//! Commit des fichiers choisis, avec message ; amender.
//!
//! Ce qui est validé est **le contenu des fichiers de travail choisis**, quel que
//! soit l'état de l'index ; les fichiers non choisis gardent leur état d'index.
//! Commandes :
//!
//! 1. `git add -- <non suivis choisis>` (sans cela `--only` refuse un chemin
//!    inconnu de Git) ;
//! 2. `git commit -F - --only [--amend] -- <choisis>` (message sur l'entrée
//!    standard ; un renommage choisi entraîne son ancien chemin).
//!
//! Si le commit échoue (crochet `pre-commit`, identité absente, signature…),
//! l'étape 1 est défaite par `git rm --cached --quiet -- <ces mêmes fichiers>` :
//! l'index redevient ce qu'il était (le disque n'est jamais touché). Les crochets
//! et la signature ne sont jamais contournés.

use std::ffi::OsString;
use std::path::PathBuf;

use julie_core::CancelToken;

use crate::error::{Result, VcsError};
use crate::git::{args, Access, Git};
use crate::repo::Repository;
use crate::status::{status, FileKind};

/// Ce qu'il faut valider.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommitRequest {
    /// Fichiers choisis (chemins absolus). Peut être vide pour un `amend` qui ne
    /// change que le message.
    pub paths: Vec<PathBuf>,
    pub message: String,
    /// Remplacer le dernier commit au lieu d'en créer un.
    pub amend: bool,
}

/// Commit créé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    pub commit: String,
    /// Première ligne du message.
    pub subject: String,
    pub amended: bool,
    /// Sortie de `git commit` (crochets compris), pour la Console.
    pub output: String,
}

/// Valide. **Bloquant.**
pub fn commit(
    git: &Git,
    repo: &Repository,
    request: &CommitRequest,
    cancel: &CancelToken,
) -> Result<CommitOutcome> {
    if request.message.trim().is_empty() {
        return Err(VcsError::EmptyMessage);
    }
    if request.paths.is_empty() && !request.amend {
        return Err(VcsError::NothingSelected);
    }
    let current = status(git, repo, cancel)?;
    if let Some(operation) = &current.operation {
        return Err(VcsError::OperationInProgress(operation.kind));
    }
    if request.amend && current.head.is_unborn() {
        return Err(VcsError::NoCommits);
    }

    let mut selected: Vec<String> = Vec::new();
    let mut to_add: Vec<String> = Vec::new();
    fn push_unique(list: &mut Vec<String>, path: String) {
        if !list.contains(&path) {
            list.push(path);
        }
    }
    for path in &request.paths {
        let repo_path = repo.to_repo_path(path)?;
        let absolute = repo.to_absolute(&repo_path);
        match current.find(&absolute) {
            Some(file) if file.kind == FileKind::Untracked => {
                push_unique(&mut to_add, repo_path.clone());
                push_unique(&mut selected, repo_path);
            }
            Some(file) => {
                push_unique(&mut selected, file.repo_path.clone());
                if let Some(original) = &file.original_repo_path {
                    push_unique(&mut selected, original.clone());
                }
            }
            // Inchangé : sans effet, mais on le laisse à Git.
            None => push_unique(&mut selected, repo_path),
        }
    }

    if !to_add.is_empty() {
        let mut command = args(["add", "--"]);
        command.extend(to_add.iter().map(OsString::from));
        git.run_ok(repo.work_tree(), &command, Access::Write, None, cancel)?;
    }

    let mut command = args(["commit", "-F", "-", "--only"]);
    if request.amend {
        command.push("--amend".into());
    }
    if !selected.is_empty() {
        command.push("--".into());
        command.extend(selected.iter().map(OsString::from));
    }
    let result = git.run_ok(
        repo.work_tree(),
        &command,
        Access::Write,
        Some(request.message.clone().into_bytes()),
        cancel,
    );
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            if !to_add.is_empty() {
                undo_add(git, repo, &to_add);
            }
            return Err(error);
        }
    };

    let head = git.run_ok(
        repo.work_tree(),
        &args(["rev-parse", "HEAD"]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(CommitOutcome {
        commit: head.stdout_text().trim().to_owned(),
        subject: request
            .message
            .trim_start()
            .lines()
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_owned(),
        amended: request.amend,
        output: format!("{}{}", output.stdout_text(), output.stderr_text()),
    })
}

/// Retire de l'index les non suivis ajoutés pour le commit. Jeton neuf : on
/// défait même si l'annulation a été demandée entre-temps.
fn undo_add(git: &Git, repo: &Repository, paths: &[String]) {
    let mut command = args(["rm", "--cached", "--quiet", "--"]);
    command.extend(paths.iter().map(OsString::from));
    let _ = git.run_ok(
        repo.work_tree(),
        &command,
        Access::Write,
        None,
        &CancelToken::new(),
    );
}
