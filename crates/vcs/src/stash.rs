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
use crate::repo::{OperationKind, Repository};

/// Début du message de toute mise de côté faite par l'IDE.
pub const AUTO_STASH_PREFIX: &str = "JulieStorm : ";

/// Fichier, **dans le dossier Git** (jamais dans l'arbre de travail : il
/// n'apparaît pas dans `git status`), qui lie une mise de côté à l'opération
/// arrêtée sur un conflit. Trois lignes : commit de l'entrée, opération
/// (`merge`, `rebase`, `cherry-pick`, `revert`), message. Relu au redémarrage
/// de l'IDE ; supprimé une fois la mise de côté remise (ou tentée).
pub const PENDING_STASH_FILE: &str = "JULIESTORM_PENDING_STASH";

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

    /// Relit un message écrit par [`StashReason::message`] (avec ou sans le
    /// « On main: » de `git stash list`). `None` si ce n'est pas une mise de
    /// côté de l'IDE ou si la forme est inconnue. Fonction pure.
    pub fn parse(message: &str) -> Option<StashReason> {
        let text = if message.starts_with(AUTO_STASH_PREFIX) {
            message
        } else {
            stash_text(message)
        };
        let text = text.strip_prefix(AUTO_STASH_PREFIX)?;
        let what = text.rsplit_once(" — ").map_or(text, |(what, _)| what);
        let what = what.strip_prefix("avant ")?;
        // Les noms de branche n'ont pas d'espace : on peut couper sur les mots.
        if let Some(rest) = what.strip_prefix("bascule de ") {
            let (from, to) = rest.split_once(" vers ")?;
            return Some(StashReason::Switch {
                from: from.to_owned(),
                to: to.to_owned(),
            });
        }
        if let Some(branch) = what.strip_prefix("mise à jour de ") {
            return Some(StashReason::Update {
                branch: branch.to_owned(),
            });
        }
        if let Some(rest) = what.strip_prefix("rebase de ") {
            let (branch, onto) = rest.split_once(" sur ")?;
            return Some(StashReason::Rebase {
                branch: branch.to_owned(),
                onto: onto.to_owned(),
            });
        }
        if let Some(rest) = what.strip_prefix("fusion de ") {
            let (revision, branch) = rest.rsplit_once(" dans ")?;
            return Some(StashReason::Merge {
                branch: branch.to_owned(),
                revision: revision.to_owned(),
            });
        }
        if let Some(branch) = what.strip_prefix("cherry-pick sur ") {
            return Some(StashReason::CherryPick {
                branch: branch.to_owned(),
            });
        }
        if let Some(branch) = what.strip_prefix("revert sur ") {
            return Some(StashReason::Revert {
                branch: branch.to_owned(),
            });
        }
        what.strip_prefix("réinitialisation de ")
            .map(|branch| StashReason::Reset {
                branch: branch.to_owned(),
            })
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
    /// Fichiers **non suivis** rangés dans l'entrée que la remise n'a pas pu
    /// rendre : leur chemin est maintenant occupé par un autre contenu (le
    /// plus souvent un fichier suivi arrivé avec la bascule ou la mise à
    /// jour). Votre version reste dans l'entrée (`git show <commit>^3:<chemin>`,
    /// ou [`crate::Revision::Commit`] de `"<commit>^3"`) ; l'entrée est alors
    /// gardée. À signaler (recette v1, m13).
    pub untracked_not_restored: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashRestore {
    /// Restaurée sans conflit ; l'entrée a été supprimée.
    Restored,
    /// Restaurée avec des conflits (marqueurs dans les fichiers) : l'entrée est
    /// **gardée** ; ouvrir la liste des conflits.
    Conflicts(Vec<PathBuf>),
    /// L'opération s'est arrêtée (conflit de fusion, de rebase…) : rien n'est
    /// encore restauré, l'entrée est gardée et **liée à l'opération** (fichier
    /// [`PENDING_STASH_FILE`] du dossier Git). Elle est remise d'elle-même par
    /// [`crate::continue_operation`] / [`crate::skip_operation`] quand
    /// l'opération se termine, et par [`crate::abort_operation`] ; si
    /// l'opération est conclue ailleurs, par [`restore_pending_stash`].
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
) -> Result<(StashRestore, Vec<PathBuf>)> {
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
    let not_restored = untracked_not_restored(git, repo, commit, cancel)?;
    if output.status.success() {
        if !not_restored.is_empty() {
            // Git dit « fait » mais un non suivi n'est pas revenu : on garde.
            return Ok((StashRestore::Failed(output_text), not_restored));
        }
        drop_stash_commit(git, repo, commit, cancel)?;
        return Ok((StashRestore::Restored, not_restored));
    }
    let conflicts = unmerged_paths(git, repo, cancel)?;
    if !conflicts.is_empty() {
        return Ok((StashRestore::Conflicts(conflicts), not_restored));
    }
    Ok((StashRestore::Failed(output_text), not_restored))
}

/// Les non suivis de l'entrée (`<commit>^3`) dont le fichier sur le disque
/// n'a pas le contenu rangé (absent ou différent). Aucun : liste vide.
fn untracked_not_restored(
    git: &Git,
    repo: &Repository,
    commit: &str,
    cancel: &CancelToken,
) -> Result<Vec<PathBuf>> {
    let third = format!("{commit}^3");
    if git
        .resolve_commit(repo.work_tree(), &third, cancel)?
        .is_none()
    {
        return Ok(Vec::new());
    }
    let listing = git.read(
        repo.work_tree(),
        &args(["ls-tree", "-r", "-z", "--full-tree", third.as_str()]),
        cancel,
    )?;
    // « <mode> blob <oid>\t<chemin> »
    let stashed: Vec<(String, String)> = listing
        .split('\0')
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let oid = meta.split_whitespace().nth(2)?;
            Some((path.to_owned(), oid.to_owned()))
        })
        .collect();
    if stashed.is_empty() {
        return Ok(Vec::new());
    }
    let mut missing = Vec::new();
    let mut present = Vec::new();
    for (path, oid) in &stashed {
        let absolute = repo.to_absolute(path);
        if absolute.is_file() {
            present.push((path, oid, absolute));
        } else {
            missing.push(absolute);
        }
    }
    if !present.is_empty() {
        // Même calcul que `git stash push` (filtres de nettoyage compris).
        let mut command = args(["hash-object", "--"]);
        command.extend(
            present
                .iter()
                .map(|(path, _, _)| OsString::from(path.as_str())),
        );
        let hashes = git.read(repo.work_tree(), &command, cancel)?;
        for ((_, oid, absolute), hash) in present.into_iter().zip(hashes.lines()) {
            if hash.trim() != oid.as_str() {
                missing.push(absolute);
            }
        }
    }
    Ok(missing)
}

/// Le lien « mise de côté ↔ opération arrêtée » écrit dans le dossier Git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingMarker {
    pub commit: String,
    pub kind: OperationKind,
    pub message: String,
}

fn kind_word(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Merge => "merge",
        OperationKind::Rebase => "rebase",
        OperationKind::CherryPick => "cherry-pick",
        OperationKind::Revert => "revert",
    }
}

pub(crate) fn read_marker(repo: &Repository) -> Option<PendingMarker> {
    let text = std::fs::read_to_string(repo.git_dir().join(PENDING_STASH_FILE)).ok()?;
    let mut lines = text.lines();
    let commit = lines.next()?.trim().to_owned();
    let kind = match lines.next()?.trim() {
        "merge" => OperationKind::Merge,
        "rebase" => OperationKind::Rebase,
        "cherry-pick" => OperationKind::CherryPick,
        "revert" => OperationKind::Revert,
        _ => return None,
    };
    let message = lines.next().unwrap_or_default().to_owned();
    (!commit.is_empty()).then_some(PendingMarker {
        commit,
        kind,
        message,
    })
}

fn write_marker(repo: &Repository, marker: &PendingMarker) -> Result<()> {
    let text = format!(
        "{}\n{}\n{}\n",
        marker.commit,
        kind_word(marker.kind),
        marker.message
    );
    julie_core::file::write_atomic(repo.git_dir().join(PENDING_STASH_FILE), text.as_bytes())?;
    Ok(())
}

fn remove_marker(repo: &Repository) {
    let _ = std::fs::remove_file(repo.git_dir().join(PENDING_STASH_FILE));
}

/// Où en est une mise de côté de l'IDE encore présente dans `git stash list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PendingStashState {
    /// Liée à l'opération en cours, arrêtée sur un conflit : elle sera
    /// **remise d'elle-même** par `continue_operation` (quand l'opération se
    /// termine), `skip_operation` (idem) ou `abort_operation`.
    AwaitingOperation(OperationKind),
    /// Liée à une opération qui s'est terminée ailleurs (terminal, autre
    /// outil) : rien ne la remettra seule ; proposer « Remettre mes
    /// changements » ([`crate::restore_pending_stash`]).
    OperationOver,
    /// Gardée (remise en conflit, refusée, « tout jeter » précédé d'une mise de
    /// côté, ou plus ancienne) : à remettre sur demande
    /// ([`restore_auto_stash`] avec son commit).
    Kept,
}

/// Une mise de côté de l'IDE en attente (bandeau « Vos changements sont mis
/// de côté »).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStash {
    pub entry: StashEntry,
    /// Le message sans « On main: » (« JulieStorm : avant mise à jour de main —
    /// … UTC »).
    pub message: String,
    /// Pourquoi elle a été faite ; `None` : message d'une forme inconnue.
    pub reason: Option<StashReason>,
    pub state: PendingStashState,
}

/// Les mises de côté de l'IDE encore dans `git stash list`, la plus récente
/// d'abord. Lit aussi le lien avec l'opération en cours (fichier
/// [`PENDING_STASH_FILE`]) : marche après un redémarrage de l'IDE. N'écrit
/// rien. **Bloquant.**
pub fn pending_auto_stashes(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<Vec<PendingStash>> {
    if !may_have_stashes(repo) {
        // Aucune référence `refs/stash` : pas de `git` à lancer (cette liste
        // est relue après chaque écriture).
        return Ok(Vec::new());
    }
    let marker = read_marker(repo);
    let operation = repo.operation().map(|operation| operation.kind);
    Ok(list_stashes(git, repo, cancel)?
        .into_iter()
        .filter(|entry| entry.automatic)
        .map(|entry| {
            let state = match &marker {
                Some(marker) if marker.commit == entry.commit => match operation {
                    Some(kind) => PendingStashState::AwaitingOperation(kind),
                    None => PendingStashState::OperationOver,
                },
                _ => PendingStashState::Kept,
            };
            let message = stash_text(&entry.message).to_owned();
            PendingStash {
                reason: StashReason::parse(&message),
                message,
                entry,
                state,
            }
        })
        .collect())
}

/// `refs/stash` peut-il exister ? Lecture du disque seulement, prudente : dans
/// le doute (format `reftable`, fichier illisible), oui.
fn may_have_stashes(repo: &Repository) -> bool {
    let git_dir = repo.git_dir();
    // Arbre lié : les références sont dans le dossier commun.
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|text| git_dir.join(text.trim()))
        .unwrap_or_else(|| git_dir.to_path_buf());
    if common.join("reftable").exists() {
        return true;
    }
    if common.join("refs").join("stash").exists()
        || common.join("logs").join("refs").join("stash").exists()
    {
        return true;
    }
    match std::fs::read_to_string(common.join("packed-refs")) {
        Ok(text) => text.lines().any(|line| line.ends_with(" refs/stash")),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Remet la mise de côté liée à une opération qui n'est plus en cours (état
/// [`PendingStashState::OperationOver`], ou appel juste après la conclusion).
/// `Ok(None)` s'il n'y en a pas ; [`VcsError::OperationInProgress`] si
/// l'opération n'est pas terminée (rien n'est fait). Le lien est retiré
/// après la tentative ; une remise en conflit garde l'entrée
/// (`StashRestore::Conflicts`). **Bloquant.**
pub fn restore_pending_stash(
    git: &Git,
    repo: &Repository,
    cancel: &CancelToken,
) -> Result<Option<AutoStash>> {
    if let Some(operation) = repo.operation() {
        return Err(VcsError::OperationInProgress(operation.kind));
    }
    let Some(marker) = read_marker(repo) else {
        return Ok(None);
    };
    let present = list_stashes(git, repo, cancel)?
        .into_iter()
        .any(|entry| entry.commit == marker.commit);
    if !present {
        // Remise ou supprimée ailleurs : le lien ne sert plus.
        remove_marker(repo);
        return Ok(None);
    }
    let result = stash_restore(git, repo, &marker.commit, cancel);
    remove_marker(repo);
    let (restore, untracked_not_restored) = result?;
    Ok(Some(AutoStash {
        message: marker.message,
        commit: marker.commit,
        restore,
        untracked_not_restored,
    }))
}

/// Après `continue` / `skip` / `abort` : si l'opération est finie, remet la
/// mise de côté liée. Ne dépend pas d'une annulation demandée entre-temps.
/// L'opération elle-même a réussi : un échec de la remise n'en fait pas un
/// échec, il est rendu en [`StashRestore::Failed`] (l'entrée reste).
pub(crate) fn after_operation(git: &Git, repo: &Repository) -> Result<Option<AutoStash>> {
    if repo.operation().is_some() {
        return Ok(None);
    }
    let marker = read_marker(repo);
    match restore_pending_stash(git, repo, &CancelToken::new()) {
        Ok(stash) => Ok(stash),
        Err(error) => Ok(marker.map(|marker| AutoStash {
            message: marker.message,
            commit: marker.commit,
            restore: StashRestore::Failed(error.to_string()),
            untracked_not_restored: Vec::new(),
        })),
    }
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
    let (restore, untracked_not_restored) = stash_restore(git, repo, commit, cancel)?;
    if repo.operation().is_none() && read_marker(repo).is_some_and(|m| m.commit == commit) {
        remove_marker(repo);
    }
    Ok(AutoStash {
        message: stash_text(&entry.message).to_owned(),
        commit: commit.to_owned(),
        restore,
        untracked_not_restored,
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
            let stopped = if finished { None } else { repo.operation() };
            let (restore, untracked_not_restored) = match stopped {
                Some(operation) => {
                    // L'opération attend : on note le lien sur le disque, pour
                    // que `continue` / `abort` remettent l'entrée, même après un
                    // redémarrage de l'IDE.
                    write_marker(
                        repo,
                        &PendingMarker {
                            commit: commit.clone(),
                            kind: operation.kind,
                            message: message.clone(),
                        },
                    )?;
                    (StashRestore::Pending, Vec::new())
                }
                None => stash_restore(git, repo, &commit, &fresh)?,
            };
            Ok((
                value,
                Some(AutoStash {
                    message,
                    commit,
                    restore,
                    untracked_not_restored,
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
