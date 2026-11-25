//! Mise de côté automatique par `git stash` (avant une bascule, une mise à
//! jour, un rebase…), et lecture des refus « vos changements locaux seraient
//! écrasés ».
//!
//! Décision `decisions-v1.md` n° 4 : en attendant l'étagère maison, les
//! changements mis de côté vont dans `git stash`, avec un message
//! reconnaissable (`JulieStorm : avant bascule de main vers x — 2026-11-25
//! 14:03 UTC`), lisible en ligne de commande. L'entrée n'est **supprimée qu'une
//! fois restaurée sans conflit** ; sinon elle reste dans `git stash list`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use julie_core::CancelToken;

use crate::conflict::unmerged_paths;
use crate::error::{OverwriteConflict, Result, VcsError};
use crate::git::{args, Access, Git};
use crate::repo::Repository;

/// Début du message de toute mise de côté faite par l'IDE.
pub const AUTO_STASH_PREFIX: &str = "JulieStorm : ";

/// Que faire des changements locaux avant une opération qui touche l'arbre de
/// travail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LocalChanges {
    /// Laisser faire Git : les changements suivent quand c'est possible ; s'ils
    /// seraient écrasés, Git refuse et l'on rend [`VcsError::WouldOverwrite`]
    /// (rien n'a changé).
    #[default]
    Keep,
    /// Mettre de côté (`git stash push --include-untracked`), opérer, puis
    /// restaurer. Rien à mettre de côté : on opère directement.
    Stash,
}

/// Pour quelle opération on met de côté (sert au message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashReason {
    /// « avant bascule de `from` vers `to` ».
    Switch { from: String, to: String },
    /// « avant mise à jour de `branch` ».
    Update { branch: String },
    /// « avant rebase de `branch` sur `onto` ».
    Rebase { branch: String, onto: String },
    /// « avant fusion de `revision` dans `branch` ».
    Merge { branch: String, revision: String },
    /// « avant cherry-pick sur `branch` ».
    CherryPick { branch: String },
    /// « avant revert sur `branch` ».
    Revert { branch: String },
    /// « avant réinitialisation de `branch` ».
    Reset { branch: String },
}

impl StashReason {
    /// Le message enregistré dans `git stash`.
    pub fn message(&self, now: SystemTime) -> String {
        let what = match self {
            StashReason::Switch { from, to } => format!("avant bascule de {from} vers {to}"),
            StashReason::Update { branch } => format!("avant mise à jour de {branch}"),
            StashReason::Rebase { branch, onto } => format!("avant rebase de {branch} sur {onto}"),
            StashReason::Merge { branch, revision } => {
                format!("avant fusion de {revision} dans {branch}")
            }
            StashReason::CherryPick { branch } => format!("avant cherry-pick sur {branch}"),
            StashReason::Revert { branch } => format!("avant revert sur {branch}"),
            StashReason::Reset { branch } => format!("avant réinitialisation de {branch}"),
        };
        format!("{AUTO_STASH_PREFIX}{what} — {}", utc_minutes(now))
    }
}

/// Ce qu'il est advenu d'une mise de côté automatique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoStash {
    /// Message de l'entrée (tel que dans `git stash list`).
    pub message: String,
    /// Commit de l'entrée : identifiant stable pour la restaurer plus tard
    /// ([`restore_auto_stash`]), même si d'autres entrées ont été ajoutées.
    pub commit: String,
    pub restore: StashRestore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashRestore {
    /// Restaurée sans conflit ; l'entrée a été supprimée.
    Restored,
    /// Restaurée avec des conflits (marqueurs dans les fichiers) : l'entrée est
    /// **gardée** ; ouvrir la liste des conflits.
    Conflicts(Vec<PathBuf>),
    /// L'opération s'est arrêtée (conflit de fusion, de rebase…) : rien n'est
    /// encore restauré, l'entrée est gardée. À restaurer une fois l'opération
    /// conclue, par [`restore_auto_stash`].
    Pending,
    /// Laissée de côté exprès (réinitialisation « tout jeter » précédée d'une
    /// mise de côté) : à restaurer quand l'utilisatrice le voudra.
    Kept,
    /// Git n'a pas pu restaurer (fichier non suivi qui serait écrasé…) :
    /// l'entrée est gardée. Texte de Git.
    Failed(String),
}

impl AutoStash {
    /// L'entrée est encore dans `git stash list`.
    pub fn kept(&self) -> bool {
        !matches!(self.restore, StashRestore::Restored)
    }
}

/// Une entrée de `git stash list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// Position (`stash@{index}`).
    pub index: usize,
    pub commit: String,
    /// Message (`On main: <message>` ou `WIP on main: …`).
    pub message: String,
    /// Faite par l'IDE (message qui commence par [`AUTO_STASH_PREFIX`]).
    pub automatic: bool,
    /// Date de création (secondes Unix).
    pub time: i64,
}

/// `git stash list`. **Bloquant.**
pub fn list_stashes(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<Vec<StashEntry>> {
    let output = git.run(
        repo.work_tree(),
        &args(["stash", "list", "-z", "--format=%H%x00%gs%x00%ct"]),
        Access::Read,
        None,
        cancel,
    )?;
    if !output.status.success() {
        // Dépôt sans commit : pas de stash possible.
        return Ok(Vec::new());
    }
    let text = output.stdout_text();
    let fields: Vec<&str> = text
        .split('\0')
        .map(|f| f.trim_start_matches('\n'))
        .collect();
    let mut entries = Vec::new();
    for (index, chunk) in fields.chunks(3).enumerate() {
        if chunk.len() < 3 || chunk[0].is_empty() {
            continue;
        }
        let message = chunk[1].to_owned();
        entries.push(StashEntry {
            index,
            commit: chunk[0].to_owned(),
            automatic: stash_text(&message).starts_with(AUTO_STASH_PREFIX),
            message,
            time: chunk[2].trim().parse().unwrap_or_default(),
        });
    }
    Ok(entries)
}

/// Le texte donné à `-m`, sans le « On main: » que Git ajoute.
fn stash_text(message: &str) -> &str {
    message
        .split_once(": ")
        .map(|(_, rest)| rest)
        .unwrap_or(message)
}

/// Met de côté tous les changements locaux (non suivis compris). `None` s'il
/// n'y avait rien. **Bloquant.**
pub(crate) fn stash_push(
    git: &Git,
    repo: &Repository,
    message: &str,
    cancel: &CancelToken,
) -> Result<Option<String>> {
    let before = git.resolve_commit(repo.work_tree(), "refs/stash", cancel)?;
    git.without_literal_pathspecs().run_ok(
        repo.work_tree(),
        &args(["stash", "push", "--include-untracked", "-m", message]),
        Access::Write,
        None,
        cancel,
    )?;
    let after = git.resolve_commit(repo.work_tree(), "refs/stash", cancel)?;
    Ok(after.filter(|after| Some(after) != before.as_ref()))
}

/// Restaure l'entrée `commit` et la supprime si
/// tout s'est bien passé. Ne rend une erreur que si Git n'a pas pu être lancé :
/// un refus ou un conflit est un [`StashRestore`]. **Bloquant.**
pub(crate) fn stash_restore(
    git: &Git,
    repo: &Repository,
    commit: &str,
    cancel: &CancelToken,
) -> Result<StashRestore> {
    // Sans `--index` : avec, Git peut restaurer les non suivis puis refuser
    // (« Conflicts in index »), et un second essai bute sur eux. Le contenu
    // revient entier ; ce qui était indexé redevient non indexé (les fichiers
    // ajoutés restent ajoutés).
    let command = args(["stash", "apply", commit]);
    let output = git.without_literal_pathspecs().run(
        repo.work_tree(),
        &command,
        Access::Write,
        None,
        cancel,
    )?;
    let output_text = format!("{}{}", output.stdout_text(), output.stderr_text());
    if output.status.success() {
        drop_stash_commit(git, repo, commit, cancel)?;
        return Ok(StashRestore::Restored);
    }
    let conflicts = unmerged_paths(git, repo, cancel)?;
    if !conflicts.is_empty() {
        return Ok(StashRestore::Conflicts(conflicts));
    }
    Ok(StashRestore::Failed(output_text))
}

/// Supprime l'entrée dont le commit est `commit` (cherchée par son commit,
/// pas par sa position). Absente : rien à faire.
fn drop_stash_commit(
    git: &Git,
    repo: &Repository,
    commit: &str,
    cancel: &CancelToken,
) -> Result<()> {
    if let Some(entry) = list_stashes(git, repo, cancel)?
        .into_iter()
        .find(|entry| entry.commit == commit)
    {
        git.run_ok(
            repo.work_tree(),
            &args([
                "stash".to_owned(),
                "drop".to_owned(),
                format!("stash@{{{}}}", entry.index),
            ]),
            Access::Write,
            None,
            cancel,
        )?;
    }
    Ok(())
}

/// Restaure une mise de côté gardée (après la conclusion d'une opération
/// arrêtée, ou après résolution). **Bloquant.**
pub fn restore_auto_stash(
    git: &Git,
    repo: &Repository,
    commit: &str,
    cancel: &CancelToken,
) -> Result<AutoStash> {
    let entry = list_stashes(git, repo, cancel)?
        .into_iter()
        .find(|entry| entry.commit == commit)
        .ok_or_else(|| VcsError::InvalidRevision(commit.to_owned()))?;
    let restore = stash_restore(git, repo, commit, cancel)?;
    Ok(AutoStash {
        message: stash_text(&entry.message).to_owned(),
        commit: commit.to_owned(),
        restore,
    })
}

/// Y a-t-il quelque chose à mettre de côté (suivi ou non suivi) ?
pub(crate) fn has_local_changes(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<bool> {
    let output = git.run_ok(
        repo.work_tree(),
        &args(["status", "--porcelain=v2", "-z", "--untracked-files=normal"]),
        Access::Read,
        None,
        cancel,
    )?;
    Ok(!output.stdout.is_empty())
}

/// Opération entourée d'une mise de côté éventuelle. `work` rend `true` si
/// l'opération est terminée (on restaure alors), `false` si elle s'est arrêtée
/// sur un conflit (l'entrée est gardée, [`StashRestore::Pending`]). Si `work`
/// échoue, on restaure aussitôt (l'opération n'a rien fait ; rien ne doit
/// rester caché), puis on rend l'erreur.
pub(crate) fn with_local_changes<T>(
    git: &Git,
    repo: &Repository,
    mode: LocalChanges,
    reason: impl FnOnce() -> StashReason,
    cancel: &CancelToken,
    work: impl FnOnce() -> Result<(T, bool)>,
) -> Result<(T, Option<AutoStash>)> {
    let stash = match mode {
        LocalChanges::Keep => None,
        LocalChanges::Stash => {
            if has_local_changes(git, repo, cancel)? {
                let message = reason().message(SystemTime::now());
                stash_push(git, repo, &message, cancel)?.map(|commit| (message, commit))
            } else {
                None
            }
        }
    };
    let result = work();
    let Some((message, commit)) = stash else {
        return result.map(|(value, _)| (value, None));
    };
    // Défaire la mise de côté ne doit pas dépendre d'une annulation demandée
    // pendant l'opération.
    let fresh = CancelToken::new();
    match result {
        Ok((value, finished)) => {
            let restore = if finished {
                stash_restore(git, repo, &commit, &fresh)?
            } else {
                StashRestore::Pending
            };
            Ok((
                value,
                Some(AutoStash {
                    message,
                    commit,
                    restore,
                }),
            ))
        }
        Err(error) => {
            let _ = stash_restore(git, repo, &commit, &fresh);
            Err(error)
        }
    }
}

/// Lit un refus « … would be overwritten by checkout/merge » de Git. `None`
/// si la sortie n'en est pas un. Fonction pure.
pub fn parse_overwrite(stderr: &str, repo: &Repository) -> Option<OverwriteConflict> {
    let mut conflict = OverwriteConflict {
        output: stderr.trim_end().to_owned(),
        ..OverwriteConflict::default()
    };
    // 0 : hors liste ; 1 : fichiers suivis ; 2 : non suivis.
    let mut list = 0;
    let mut found = false;
    for line in stderr.lines() {
        if let Some(path) = line.strip_prefix('\t') {
            let path = repo.to_absolute(&unquote(path.trim_end()));
            match list {
                1 => conflict.tracked.push(path),
                2 => conflict.untracked.push(path),
                _ => {}
            }
            continue;
        }
        list = 0;
        let lower = line.to_ascii_lowercase();
        if lower.contains("would be overwritten by") || lower.contains("would be removed by") {
            found = true;
            list = if lower.contains("untracked") { 2 } else { 1 };
        }
    }
    found.then_some(conflict)
}

/// Retire les guillemets C que Git met autour d'un chemin inhabituel
/// (`"a\"b"`, `"\303\251"`). Un chemin sans guillemets est rendu tel quel.
pub(crate) fn unquote(path: &str) -> String {
    let Some(inner) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
        return path.to_owned();
    };
    let mut bytes = Vec::new();
    let mut chars = inner.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match chars.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(b'r') => bytes.push(b'\r'),
            Some(b'a') => bytes.push(7),
            Some(b'b') => bytes.push(8),
            Some(b'f') => bytes.push(12),
            Some(b'v') => bytes.push(11),
            Some(digit @ b'0'..=b'7') => {
                let mut value = u32::from(digit - b'0');
                for _ in 0..2 {
                    match chars.peek() {
                        Some(next @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(next - b'0');
                            chars.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(value as u8);
            }
            Some(other) => bytes.push(other),
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `AAAA-MM-JJ HH:MM UTC` (pas de fuseau local sans dépendance de plus).
fn utc_minutes(now: SystemTime) -> String {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Algorithme « civil from days » (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// Chemins Git d'une liste de chemins absolus, pour une commande (la racine
/// elle-même devient `.`).
pub(crate) fn repo_paths(repo: &Repository, paths: &[PathBuf]) -> Result<Vec<OsString>> {
    paths
        .iter()
        .map(|path| {
            if repo.is_root(path) {
                Ok(OsString::from("."))
            } else {
                repo.to_repo_path(path).map(OsString::from)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dates_are_written_in_utc() {
        let time = UNIX_EPOCH + Duration::from_secs(1_795_000_000);
        assert_eq!(utc_minutes(time), "2026-11-18 11:06 UTC");
        assert_eq!(utc_minutes(UNIX_EPOCH), "1970-01-01 00:00 UTC");
    }

    #[test]
    fn auto_stash_messages_are_recognisable() {
        let message = StashReason::Switch {
            from: "main".into(),
            to: "feature/x".into(),
        }
        .message(UNIX_EPOCH);
        assert_eq!(
            message,
            "JulieStorm : avant bascule de main vers feature/x — 1970-01-01 00:00 UTC"
        );
        assert!(stash_text(&format!("On main: {message}")).starts_with(AUTO_STASH_PREFIX));
    }

    #[test]
    fn overwrite_refusals_name_their_files() {
        let repo = crate::repo::tests_support::repository(r"C:\d");
        let stderr = "error: Your local changes to the following files would be overwritten by checkout:\n\
                      \ta.txt\n\
                      \tdossier é/b c.txt\n\
                      Please commit your changes or stash them before you switch branches.\n\
                      error: The following untracked working tree files would be overwritten by checkout:\n\
                      \t\"guil\\\"lemet.txt\"\n\
                      Please move or remove them before you switch branches.\n\
                      Aborting\n";
        let conflict = parse_overwrite(stderr, &repo).unwrap();
        assert_eq!(
            conflict.tracked,
            vec![
                PathBuf::from(r"C:\d\a.txt"),
                PathBuf::from(r"C:\d\dossier é\b c.txt")
            ]
        );
        assert_eq!(
            conflict.untracked,
            vec![PathBuf::from(r#"C:\d\guil"lemet.txt"#)]
        );
        assert!(parse_overwrite("fatal: autre chose", &repo).is_none());
    }

    #[test]
    fn c_quoted_paths_are_decoded() {
        assert_eq!(unquote("\"\\303\\251t\\303\\251.txt\""), "été.txt");
        assert_eq!(unquote("simple.txt"), "simple.txt");
        assert_eq!(unquote("\"a\\tb\""), "a\tb");
    }
}
