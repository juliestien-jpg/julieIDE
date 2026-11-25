//! Le distant : récupérer (fetch), mettre à jour le projet, tirer (pull),
//! pousser (push), supprimer une branche distante, avancer une branche non
//! courante.
//!
//! Aucune saisie d'identifiants : `GIT_TERMINAL_PROMPT=0` (posé par [`Git`]).
//! Le gestionnaire d'identifiants de Git et l'agent SSH font leur travail ;
//! si Git a besoin d'un mot de passe qu'il ne peut pas demander, l'opération
//! échoue en [`VcsError::AuthenticationFailed`]. La progression arrive par
//! un rappel ([`RemoteProgress`]) ; l'annulation tue Git et ses descendants.
//! Aucune commande réseau n'est lancée sans demande de l'appelant.

use std::ffi::OsString;

use julie_core::CancelToken;

use crate::branches::is_ancestor;
use crate::content::check_revision;
use crate::error::{PushRejection, Result, VcsError};
use crate::git::{args, failure, is_protected, Git, IntegrationMethod};
use crate::history::{run_log, CommitSummary};
use crate::operations::{finish_outcome, run_stoppable, OperationOutcome};
use crate::repo::{current_branch_name, OperationKind, Repository};
use crate::stash::{with_local_changes, LocalChanges, StashReason};

// ------------------------------------------------------------------ progression

/// Étape d'une opération réseau, telle que Git la nomme.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProgressPhase {
    Enumerating,
    Counting,
    Compressing,
    Receiving,
    Resolving,
    Writing,
    Checkout,
    /// Autre libellé de Git (texte brut).
    Other(String),
}

/// Une ligne de progression de Git (`Receiving objects:  45% (9/20)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteProgress {
    pub phase: ProgressPhase,
    /// Ligne venue du serveur (`remote: …`).
    pub from_remote: bool,
    pub percent: Option<u8>,
    pub done: u64,
    /// 0 si inconnu.
    pub total: u64,
}

/// Découpe la sortie d'erreur de Git (lignes séparées par `\r` ou `\n`) en
/// progressions. Garde le morceau de ligne incomplet d'un appel à l'autre.
#[derive(Debug, Default)]
pub struct ProgressParser {
    pending: Vec<u8>,
}

impl ProgressParser {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<RemoteProgress> {
        self.pending.extend_from_slice(chunk);
        let mut results = Vec::new();
        while let Some(end) = self.pending.iter().position(|&b| b == b'\r' || b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            if let Some(progress) = parse_progress_line(&String::from_utf8_lossy(&line)) {
                results.push(progress);
            }
        }
        results
    }
}

/// Une ligne de progression, ou `None`. Fonction pure.
pub fn parse_progress_line(line: &str) -> Option<RemoteProgress> {
    let line = line.trim();
    let (from_remote, line) = match line.strip_prefix("remote:") {
        Some(rest) => (true, rest.trim()),
        None => (false, line),
    };
    let (label, rest) = line.split_once(": ")?;
    let rest = rest.trim();
    let percent = rest
        .split_once('%')
        .and_then(|(number, _)| number.trim().parse::<u8>().ok());
    let (done, total) = match rest.split_once('(').and_then(|(_, r)| r.split_once(')')) {
        Some((inside, _)) => {
            let (a, b) = inside.split_once('/')?;
            (a.trim().parse().ok()?, b.trim().parse().ok()?)
        }
        None => {
            let number: String = rest.chars().take_while(char::is_ascii_digit).collect();
            (number.parse().ok()?, 0)
        }
    };
    let phase = match label {
        "Enumerating objects" => ProgressPhase::Enumerating,
        "Counting objects" => ProgressPhase::Counting,
        "Compressing objects" => ProgressPhase::Compressing,
        "Receiving objects" => ProgressPhase::Receiving,
        "Resolving deltas" => ProgressPhase::Resolving,
        "Writing objects" => ProgressPhase::Writing,
        "Updating files" | "Checking out files" => ProgressPhase::Checkout,
        other if percent.is_some() => ProgressPhase::Other(other.to_owned()),
        _ => return None,
    };
    Some(RemoteProgress {
        phase,
        from_remote,
        percent,
        done,
        total,
    })
}

/// Lance une commande réseau en passant sa progression à `on_progress`.
fn run_network(
    git: &Git,
    repo: &Repository,
    command: &[OsString],
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<julie_core::process::Output> {
    let mut parser = ProgressParser::default();
    git.run_streaming(repo.work_tree(), command, cancel, &mut |chunk| {
        for progress in parser.feed(chunk) {
            on_progress(progress);
        }
    })
}

// ------------------------------------------------------------------ fetch

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest {
    /// Un distant ; `None` : tous (`--all`).
    pub remote: Option<String>,
    /// Retirer les branches distantes supprimées sur le distant (`--prune`).
    pub prune: bool,
}

impl Default for FetchRequest {
    fn default() -> Self {
        Self {
            remote: None,
            prune: true,
        }
    }
}

/// Ce que le fetch a changé à une référence (`git fetch --porcelain`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefUpdateKind {
    FastForward,
    Forced,
    Pruned,
    TagUpdate,
    New,
    Rejected,
    UpToDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub kind: RefUpdateKind,
    pub old: Option<String>,
    pub new: Option<String>,
    /// `refs/remotes/origin/main`.
    pub local_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchOutcome {
    pub updates: Vec<RefUpdate>,
    pub output: String,
}

impl FetchOutcome {
    /// Références créées ou avancées (« 3 branches mises à jour »).
    pub fn updated(&self) -> usize {
        self.updates
            .iter()
            .filter(|u| {
                matches!(
                    u.kind,
                    RefUpdateKind::FastForward
                        | RefUpdateKind::Forced
                        | RefUpdateKind::New
                        | RefUpdateKind::TagUpdate
                )
            })
            .count()
    }

    /// Branches distantes retirées (« 1 supprimée sur le distant »).
    pub fn pruned(&self) -> usize {
        self.updates
            .iter()
            .filter(|u| u.kind == RefUpdateKind::Pruned)
            .count()
    }

    /// « Rien de nouveau ».
    pub fn is_empty(&self) -> bool {
        self.updated() == 0 && self.pruned() == 0
    }
}

/// Lit la sortie de `git fetch --porcelain`. Fonction pure.
pub fn parse_fetch_porcelain(text: &str) -> Vec<RefUpdate> {
    let zero = |oid: &str| oid.bytes().all(|b| b == b'0');
    text.lines()
        .filter_map(|line| {
            let flag = line.chars().next()?;
            let mut parts = line.get(2..)?.split(' ');
            let old = parts.next()?;
            let new = parts.next()?;
            let local_ref = parts.next()?.to_owned();
            let kind = match flag {
                ' ' => RefUpdateKind::FastForward,
                '+' => RefUpdateKind::Forced,
                '-' => RefUpdateKind::Pruned,
                't' => RefUpdateKind::TagUpdate,
                '*' => RefUpdateKind::New,
                '!' => RefUpdateKind::Rejected,
                '=' => RefUpdateKind::UpToDate,
                _ => return None,
            };
            Some(RefUpdate {
                kind,
                old: (!zero(old)).then(|| old.to_owned()),
                new: (!zero(new)).then(|| new.to_owned()),
                local_ref,
            })
        })
        .collect()
}

/// `git fetch --porcelain --progress [--prune] (--all | <distant>)`.
/// **Bloquant** ; pas de délai maximal, annulable.
pub fn fetch(
    git: &Git,
    repo: &Repository,
    request: &FetchRequest,
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<FetchOutcome> {
    let mut command = args(["fetch", "--porcelain", "--progress"]);
    if request.prune {
        command.push("--prune".into());
    }
    match &request.remote {
        Some(remote) => {
            check_revision(remote)?;
            command.push(remote.into());
        }
        None => command.push("--all".into()),
    }
    let output = run_network(git, repo, &command, on_progress, cancel)?;
    if !output.status.success() {
        return Err(failure(&command, &output));
    }
    Ok(FetchOutcome {
        updates: parse_fetch_porcelain(&output.stdout_text()),
        output: format!("{}{}", output.stdout_text(), output.stderr_text()),
    })
}

// ------------------------------------------------------------------ amont

/// L'amont d'une branche locale, tel que configuré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpstreamConfig {
    /// `refs/remotes/origin/main`.
    pub full: String,
    /// `origin/main`.
    pub short: String,
    /// `origin` (`.` pour un amont local).
    pub remote: String,
    /// `refs/heads/main` (sur le distant).
    pub remote_ref: String,
}

pub(crate) fn upstream_config(
    git: &Git,
    repo: &Repository,
    branch: &str,
    cancel: &CancelToken,
) -> Result<Option<UpstreamConfig>> {
    check_revision(branch)?;
    let text = git.read(
        repo.work_tree(),
        &args([
            "for-each-ref".to_owned(),
            "--format=%(upstream)%00%(upstream:short)%00%(upstream:remotename)%00%(upstream:remoteref)".to_owned(),
            format!("refs/heads/{branch}"),
        ]),
        cancel,
    )?;
    let line = text.lines().next().unwrap_or_default();
    let fields: Vec<&str> = line.split('\0').collect();
    if fields.len() < 4 || fields[0].is_empty() {
        return Ok(None);
    }
    Ok(Some(UpstreamConfig {
        full: fields[0].to_owned(),
        short: fields[1].to_owned(),
        remote: fields[2].to_owned(),
        remote_ref: fields[3].to_owned(),
    }))
}

fn current_branch_required(git: &Git, repo: &Repository, cancel: &CancelToken) -> Result<String> {
    let name = current_branch_name(repo.git_dir()).ok_or(VcsError::DetachedHead)?;
    if git
        .resolve_commit(repo.work_tree(), "HEAD", cancel)?
        .is_none()
    {
        return Err(VcsError::NoCommits);
    }
    Ok(name)
}

// ------------------------------------------------------------------ mettre à jour, pull

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    pub method: IntegrationMethod,
    /// Défaut de « Mettre à jour le projet » : [`LocalChanges::Stash`]
    /// (« avant mise à jour »).
    pub local_changes: LocalChanges,
}

/// « Mettre à jour le projet » (Ctrl+T) : fetch du distant de l'amont, puis
/// `git merge @{u}` ou `git rebase @{u}`, entouré d'une mise de côté. Pas
/// d'amont : [`VcsError::NoUpstream`] **sans aucune commande réseau ni
/// fusion** ; amont supprimé : [`VcsError::UpstreamGone`]. **Bloquant.**
pub fn update_project(
    git: &Git,
    repo: &Repository,
    request: &UpdateRequest,
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    if let Some(operation) = repo.operation() {
        return Err(VcsError::OperationInProgress(operation.kind));
    }
    let branch = current_branch_required(git, repo, cancel)?;
    let upstream =
        upstream_config(git, repo, &branch, cancel)?.ok_or_else(|| VcsError::NoUpstream {
            branch: branch.clone(),
        })?;
    let fetched = if upstream.remote != "." {
        Some(fetch(
            git,
            repo,
            &FetchRequest {
                remote: Some(upstream.remote.clone()),
                prune: true,
            },
            on_progress,
            cancel,
        )?)
    } else {
        None
    };
    let tip = git
        .resolve_commit(repo.work_tree(), &upstream.full, cancel)?
        .ok_or_else(|| VcsError::UpstreamGone {
            branch: branch.clone(),
            upstream: upstream.short.clone(),
        })?;
    let kind = match request.method {
        IntegrationMethod::Merge => OperationKind::Merge,
        IntegrationMethod::Rebase => OperationKind::Rebase,
    };
    let before = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    let mut outcome = OperationOutcome::new(kind, before.clone());
    outcome.fetch = fetched;
    let before = before.unwrap_or_default();
    if is_ancestor(git, repo, &tip, &before, cancel)? {
        outcome.up_to_date = true;
        return Ok(outcome);
    }
    outcome.received = git.count(repo.work_tree(), &[&tip, &format!("^{before}")], cancel)?;
    let command = match request.method {
        IntegrationMethod::Merge => args(["merge", "--no-edit", upstream.short.as_str()]),
        IntegrationMethod::Rebase => args(["rebase", upstream.short.as_str()]),
    };
    let ((text, finished), stash) = with_local_changes(
        git,
        repo,
        request.local_changes,
        || StashReason::Update {
            branch: branch.clone(),
        },
        cancel,
        || run_stoppable(git, repo, &command, cancel).map(|(t, f)| ((t, f), f)),
    )?;
    outcome.stash = stash;
    finish_outcome(git, repo, &mut outcome, finished, text, cancel)?;
    outcome.fast_forward = finished && outcome.head.as_deref() == Some(tip.as_str());
    Ok(outcome)
}

/// Mode de `git pull`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PullMode {
    /// `--no-rebase`.
    Merge,
    /// `--rebase`.
    Rebase,
    /// `--ff-only` (« Avance rapide seulement »).
    FastForwardOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub remote: String,
    /// Branche du distant (`main`).
    pub branch: String,
    pub mode: PullMode,
    pub local_changes: LocalChanges,
}

/// `git pull --progress <mode> <distant> <branche>` dans la branche courante.
/// **Bloquant.**
pub fn pull(
    git: &Git,
    repo: &Repository,
    request: &PullRequest,
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<OperationOutcome> {
    if let Some(operation) = repo.operation() {
        return Err(VcsError::OperationInProgress(operation.kind));
    }
    check_revision(&request.remote)?;
    check_revision(&request.branch)?;
    let branch = current_branch_required(git, repo, cancel)?;
    let kind = match request.mode {
        PullMode::Rebase => OperationKind::Rebase,
        _ => OperationKind::Merge,
    };
    let before = git.resolve_commit(repo.work_tree(), "HEAD", cancel)?;
    let mut outcome = OperationOutcome::new(kind, before.clone());
    let mut command = args(["pull", "--progress"]);
    command.extend(
        match request.mode {
            PullMode::Merge => vec!["--no-rebase", "--no-edit"],
            PullMode::Rebase => vec!["--rebase"],
            PullMode::FastForwardOnly => vec!["--no-rebase", "--ff-only"],
        }
        .into_iter()
        .map(OsString::from),
    );
    command.push(request.remote.as_str().into());
    command.push(request.branch.as_str().into());

    let ((text, finished), stash) = with_local_changes(
        git,
        repo,
        request.local_changes,
        || StashReason::Update {
            branch: branch.clone(),
        },
        cancel,
        || {
            let output = run_network(git, repo, &command, on_progress, cancel)?;
            let text = format!("{}{}", output.stdout_text(), output.stderr_text());
            if output.status.success() {
                let finished = repo.operation().is_none();
                return Ok(((text, finished), finished));
            }
            if repo.operation().is_some() {
                return Ok(((text, false), false));
            }
            if let Some(conflict) = crate::stash::parse_overwrite(&output.stderr_text(), repo) {
                return Err(VcsError::WouldOverwrite(conflict));
            }
            Err(failure(&command, &output))
        },
    )?;
    outcome.stash = stash;
    outcome.up_to_date = finished && text.contains("Already up to date");
    if let (Some(before), Some(fetched)) = (
        before,
        git.resolve_commit(repo.work_tree(), "FETCH_HEAD", cancel)?,
    ) {
        outcome.received =
            git.count(repo.work_tree(), &[&fetched, &format!("^{before}")], cancel)?;
    }
    finish_outcome(git, repo, &mut outcome, finished, text, cancel)?;
    Ok(outcome)
}

/// « Mettre à jour » une branche locale **non courante** : avance rapide sur
/// son amont sans basculer (`git fetch <distant> <réf>:refs/heads/<branche>`).
/// Divergée : [`VcsError::Diverged`], rien ne change. Courante :
/// [`VcsError::CurrentBranch`] (passer par [`update_project`]). **Bloquant.**
pub fn fast_forward_branch(
    git: &Git,
    repo: &Repository,
    branch: &str,
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<FetchOutcome> {
    if current_branch_name(repo.git_dir()).as_deref() == Some(branch) {
        return Err(VcsError::CurrentBranch(branch.to_owned()));
    }
    let upstream =
        upstream_config(git, repo, branch, cancel)?.ok_or_else(|| VcsError::NoUpstream {
            branch: branch.to_owned(),
        })?;
    if upstream.remote == "." {
        return Err(VcsError::InvalidRequest("amont local".into()));
    }
    let command = args([
        "fetch".to_owned(),
        "--porcelain".to_owned(),
        "--progress".to_owned(),
        upstream.remote.clone(),
        format!("{}:refs/heads/{branch}", upstream.remote_ref),
        format!("{}:{}", upstream.remote_ref, upstream.full),
    ]);
    let output = run_network(git, repo, &command, on_progress, cancel)?;
    let updates = parse_fetch_porcelain(&output.stdout_text());
    let local = format!("refs/heads/{branch}");
    if updates
        .iter()
        .any(|u| u.kind == RefUpdateKind::Rejected && u.local_ref == local)
        || output.stderr_text().contains("non-fast-forward")
    {
        return Err(VcsError::Diverged {
            branch: branch.to_owned(),
        });
    }
    if !output.status.success() {
        if output.stderr_text().contains("couldn't find remote ref") {
            return Err(VcsError::UpstreamGone {
                branch: branch.to_owned(),
                upstream: upstream.short,
            });
        }
        return Err(failure(&command, &output));
    }
    Ok(FetchOutcome {
        updates,
        output: format!("{}{}", output.stdout_text(), output.stderr_text()),
    })
}

// ------------------------------------------------------------------ push

/// Ce que l'on pousse, et où.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PushTarget {
    /// Branche locale.
    pub branch: String,
    pub remote: String,
    /// Nom de la branche sur le distant.
    pub remote_branch: String,
}

/// La cible par défaut d'une branche (la courante si `None`) : son amont,
/// sinon `origin` (ou le seul distant) et le même nom. **Bloquant.**
pub fn push_target(
    git: &Git,
    repo: &Repository,
    branch: Option<&str>,
    cancel: &CancelToken,
) -> Result<PushTarget> {
    let branch = match branch {
        Some(name) => name.to_owned(),
        None => current_branch_required(git, repo, cancel)?,
    };
    if let Some(upstream) = upstream_config(git, repo, &branch, cancel)? {
        if let Some(remote_branch) = upstream.remote_ref.strip_prefix("refs/heads/") {
            if upstream.remote != "." {
                return Ok(PushTarget {
                    remote_branch: remote_branch.to_owned(),
                    remote: upstream.remote,
                    branch,
                });
            }
        }
    }
    let remotes = crate::branches::list_remotes(git, repo, cancel)?;
    let remote = if remotes.iter().any(|r| r == "origin") {
        "origin".to_owned()
    } else {
        remotes
            .into_iter()
            .next()
            .ok_or_else(|| VcsError::InvalidRequest("aucun distant".into()))?
    };
    Ok(PushTarget {
        remote_branch: branch.clone(),
        remote,
        branch,
    })
}

/// Ce que montre le dialogue de push avant l'envoi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushPreview {
    pub target: PushTarget,
    /// La branche n'existe pas encore sur le distant (« nouvelle ») — d'après
    /// le dernier fetch.
    pub new_branch: bool,
    /// Commits sortants, du plus récent au plus ancien (`amont..branche`, ou
    /// pour une nouvelle branche `branche --not --remotes`). Vide : « Rien à
    /// pousser ».
    pub outgoing: Vec<CommitSummary>,
    /// Sommet connu de la branche distante (`refs/remotes/<distant>/<nom>`).
    pub remote_tip: Option<String>,
    /// Commits connus du distant absents de la branche : ceux qu'un « Forcer
    /// avec bail » remplacerait.
    pub remote_only: u32,
    /// Branche protégée : « Forcer avec bail » grisé.
    pub protected: bool,
    /// La branche locale n'a pas d'amont : pousser le posera (`-u`).
    pub sets_upstream: bool,
}

/// Prépare le dialogue de push. Aucune commande réseau. **Bloquant.**
pub fn push_preview(
    git: &Git,
    repo: &Repository,
    target: &PushTarget,
    protected: &[String],
    cancel: &CancelToken,
) -> Result<PushPreview> {
    check_target(target)?;
    let local = format!("refs/heads/{}", target.branch);
    git.resolve_commit(repo.work_tree(), &local, cancel)?
        .ok_or_else(|| VcsError::BranchNotFound(target.branch.clone()))?;
    let remote_ref = format!("refs/remotes/{}/{}", target.remote, target.remote_branch);
    let remote_tip = git.resolve_commit(repo.work_tree(), &remote_ref, cancel)?;
    let mut options: Vec<OsString> = vec!["--max-count=1000".into(), "--date-order".into()];
    let remote_only = match &remote_tip {
        Some(tip) => {
            options.push(local.as_str().into());
            options.push(format!("^{tip}").into());
            git.count(repo.work_tree(), &[tip, &format!("^{local}")], cancel)?
        }
        None => {
            options.push(local.as_str().into());
            options.push("--not".into());
            options.push("--remotes".into());
            0
        }
    };
    options.push("--".into());
    let outgoing = run_log(git, repo, options, cancel)?;
    Ok(PushPreview {
        new_branch: remote_tip.is_none(),
        outgoing,
        remote_only,
        protected: is_protected(protected, &target.remote_branch),
        sets_upstream: upstream_config(git, repo, &target.branch, cancel)?.is_none(),
        remote_tip,
        target: target.clone(),
    })
}

fn check_target(target: &PushTarget) -> Result<()> {
    check_revision(&target.branch)?;
    check_revision(&target.remote)?;
    check_revision(&target.remote_branch)
}

/// Forcer ou non.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum PushForce {
    #[default]
    No,
    /// `--force-with-lease=refs/heads/<nom>:<attendu>` : refusé si le distant a
    /// bougé depuis `expected`. `None` : le sommet connu localement de la
    /// branche distante (ou « n'existe pas » si on ne la connaît pas).
    /// Jamais de `--force` nu. Refusé sur une branche protégée.
    WithLease { expected: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRequest {
    pub target: PushTarget,
    /// Poser l'amont (`-u`) ; à cocher pour une nouvelle branche.
    pub set_upstream: bool,
    /// « Pousser aussi les étiquettes » (`--follow-tags`).
    pub follow_tags: bool,
    pub force: PushForce,
}

/// Ce que le push a fait à la branche distante.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PushedKind {
    New,
    FastForward,
    Forced,
    UpToDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushOutcome {
    pub target: PushTarget,
    pub kind: PushedKind,
    /// Commits envoyés (« 3 commits poussés vers origin/C »).
    pub pushed: u32,
    pub upstream_set: bool,
    pub output: String,
}

/// Une ligne de `git push --porcelain` : `drapeau \t de:vers \t résumé`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PushLine {
    pub flag: char,
    pub to: String,
    pub summary: String,
}

pub(crate) fn parse_push_porcelain(text: &str) -> Vec<PushLine> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let flag = parts.next()?;
            let refs = parts.next()?;
            let summary = parts.next().unwrap_or_default();
            if flag.chars().count() != 1 {
                return None;
            }
            let to = refs.rsplit_once(':').map(|(_, to)| to).unwrap_or(refs);
            Some(PushLine {
                flag: flag.chars().next()?,
                to: to.to_owned(),
                summary: summary.to_owned(),
            })
        })
        .collect()
}

/// Analyse un résumé de refus (`[rejected] (fetch first)`…). Fonction pure.
pub fn classify_rejection(summary: &str, known_behind: u32) -> PushRejection {
    let reason = summary
        .split_once('(')
        .and_then(|(_, r)| r.rsplit_once(')'))
        .map(|(r, _)| r.trim().to_owned())
        .unwrap_or_else(|| summary.trim().to_owned());
    if summary.contains("[remote rejected]") {
        return PushRejection::RemoteRejected { reason };
    }
    match reason.as_str() {
        "non-fast-forward" | "fetch first" => PushRejection::NonFastForward { known_behind },
        "stale info" => PushRejection::StaleLease,
        _ => PushRejection::Other { reason },
    }
}

/// `git push --porcelain --progress [-u] [--follow-tags] [--force-with-lease=…]
/// <distant> refs/heads/<branche>:refs/heads/<nom distant>`. Refus analysés
/// ([`VcsError::PushRejected`]) ; crochet `pre-push` en échec :
/// [`VcsError::GitFailed`] avec sa sortie ; rien n'est réessayé. **Bloquant.**
pub fn push(
    git: &Git,
    repo: &Repository,
    request: &PushRequest,
    protected: &[String],
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<PushOutcome> {
    let target = &request.target;
    check_target(target)?;
    let local = format!("refs/heads/{}", target.branch);
    let local_tip = git
        .resolve_commit(repo.work_tree(), &local, cancel)?
        .ok_or_else(|| VcsError::BranchNotFound(target.branch.clone()))?;
    let remote_tracking = format!("refs/remotes/{}/{}", target.remote, target.remote_branch);
    let remote_tip = git.resolve_commit(repo.work_tree(), &remote_tracking, cancel)?;
    let pushed = match &remote_tip {
        Some(tip) => git.count(repo.work_tree(), &[&local_tip, &format!("^{tip}")], cancel)?,
        None => git.count(
            repo.work_tree(),
            &[&local_tip, "--not", "--remotes"],
            cancel,
        )?,
    };
    let destination = format!("refs/heads/{}", target.remote_branch);

    let mut command = args(["push", "--porcelain", "--progress"]);
    if request.set_upstream {
        command.push("--set-upstream".into());
    }
    if request.follow_tags {
        command.push("--follow-tags".into());
    }
    if let PushForce::WithLease { expected } = &request.force {
        if is_protected(protected, &target.remote_branch) {
            return Err(VcsError::ProtectedBranch(target.remote_branch.clone()));
        }
        let expected = match expected {
            Some(sha) => {
                check_revision(sha)?;
                sha.clone()
            }
            None => remote_tip.clone().unwrap_or_default(),
        };
        command.push(format!("--force-with-lease={destination}:{expected}").into());
    }
    command.push(target.remote.as_str().into());
    command.push(format!("{local}:{destination}").into());

    let output = run_network(git, repo, &command, on_progress, cancel)?;
    let text = format!("{}{}", output.stdout_text(), output.stderr_text());
    let lines = parse_push_porcelain(&output.stdout_text());
    let line = lines.iter().find(|line| line.to == destination);
    if !output.status.success() {
        if let Some(line) = line.filter(|line| line.flag == '!') {
            let known_behind = match &remote_tip {
                Some(tip) => {
                    git.count(repo.work_tree(), &[tip, &format!("^{local_tip}")], cancel)?
                }
                None => 0,
            };
            return Err(VcsError::PushRejected {
                rejection: classify_rejection(&line.summary, known_behind),
                output: text,
            });
        }
        return Err(failure(&command, &output));
    }
    let kind = match line.map(|line| line.flag) {
        Some('*') => PushedKind::New,
        Some('+') => PushedKind::Forced,
        Some('=') => PushedKind::UpToDate,
        _ => PushedKind::FastForward,
    };
    Ok(PushOutcome {
        target: target.clone(),
        kind,
        pushed: if kind == PushedKind::UpToDate {
            0
        } else {
            pushed
        },
        upstream_set: request.set_upstream,
        output: text,
    })
}

/// Branche distante supprimée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBranchDeleted {
    pub remote: String,
    pub branch: String,
    /// Son dernier sommet connu (pour la recréer en la poussant).
    pub tip: Option<String>,
    pub output: String,
}

/// **Destructif, pour tout le monde** : `git push --porcelain <distant>
/// --delete refs/heads/<branche>`, après une confirmation forte. Refusé sur
/// une branche protégée. **Bloquant.**
pub fn delete_remote_branch(
    git: &Git,
    repo: &Repository,
    remote: &str,
    branch: &str,
    protected: &[String],
    on_progress: &mut dyn FnMut(RemoteProgress),
    cancel: &CancelToken,
) -> Result<RemoteBranchDeleted> {
    check_revision(remote)?;
    check_revision(branch)?;
    if is_protected(protected, branch) {
        return Err(VcsError::ProtectedBranch(branch.to_owned()));
    }
    let tip = git.resolve_commit(
        repo.work_tree(),
        &format!("refs/remotes/{remote}/{branch}"),
        cancel,
    )?;
    let command = args([
        "push".to_owned(),
        "--porcelain".to_owned(),
        "--progress".to_owned(),
        remote.to_owned(),
        "--delete".to_owned(),
        format!("refs/heads/{branch}"),
    ]);
    let output = run_network(git, repo, &command, on_progress, cancel)?;
    let text = format!("{}{}", output.stdout_text(), output.stderr_text());
    if !output.status.success() {
        if let Some(line) = parse_push_porcelain(&output.stdout_text())
            .into_iter()
            .find(|line| line.flag == '!')
        {
            return Err(VcsError::PushRejected {
                rejection: classify_rejection(&line.summary, 0),
                output: text,
            });
        }
        return Err(failure(&command, &output));
    }
    Ok(RemoteBranchDeleted {
        remote: remote.to_owned(),
        branch: branch.to_owned(),
        tip,
        output: text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_are_read_across_chunks() {
        let mut parser = ProgressParser::default();
        let mut all = parser.feed(b"remote: Counting objects:  50% (3/6)\rremote: Coun");
        all.extend(
            parser.feed(b"ting objects: 100% (6/6), done.\nReceiving objects:  45% (9/20)\r"),
        );
        all.extend(parser.feed(b"Resolving deltas: 100% (2/2), done.\nTo C:/x\n"));
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].phase, ProgressPhase::Counting);
        assert!(all[0].from_remote);
        assert_eq!(
            (all[0].done, all[0].total, all[0].percent),
            (3, 6, Some(50))
        );
        assert_eq!(all[2].phase, ProgressPhase::Receiving);
        assert!(!all[2].from_remote);
        assert_eq!(all[3].phase, ProgressPhase::Resolving);
        assert_eq!(
            parse_progress_line("remote: Enumerating objects: 6, done.").map(|p| (p.done, p.total)),
            Some((6, 0))
        );
        assert!(parse_progress_line("hint: Updates were rejected").is_none());
    }

    #[test]
    fn fetch_porcelain_lines_are_parsed() {
        let text = "* 0000000000000000000000000000000000000000 50e93e4541d4da0839f6ca8f7cfdadba27463db6 refs/remotes/origin/main\n\
                    - cedeb222e2c2f89efa56419f117c0a420bb66e66 0000000000000000000000000000000000000000 refs/remotes/origin/feature/login\n\
                    \x20 50e93e4541d4da0839f6ca8f7cfdadba27463db6 72e0a98dc1fb7aba394526bf511cc39a5aba7355 refs/remotes/origin/main\n";
        let updates = parse_fetch_porcelain(text);
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0].kind, RefUpdateKind::New);
        assert_eq!(updates[0].old, None);
        assert_eq!(updates[1].kind, RefUpdateKind::Pruned);
        assert_eq!(updates[1].new, None);
        assert_eq!(updates[2].kind, RefUpdateKind::FastForward);
        let outcome = FetchOutcome {
            updates,
            output: String::new(),
        };
        assert_eq!((outcome.updated(), outcome.pruned()), (2, 1));
    }

    #[test]
    fn push_refusals_are_classified() {
        let lines = parse_push_porcelain(
            "To C:/d.git\n!\trefs/heads/main:refs/heads/main\t[rejected] (fetch first)\nDone\n",
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].to, "refs/heads/main");
        assert_eq!(
            classify_rejection(&lines[0].summary, 2),
            PushRejection::NonFastForward { known_behind: 2 }
        );
        assert_eq!(
            classify_rejection("[rejected] (stale info)", 0),
            PushRejection::StaleLease
        );
        assert_eq!(
            classify_rejection("[remote rejected] (pre-receive hook declined)", 0),
            PushRejection::RemoteRejected {
                reason: "pre-receive hook declined".into()
            }
        );
        let deleted = parse_push_porcelain("-\t:refs/heads/feature/login\t[deleted]\n");
        assert_eq!(deleted[0].flag, '-');
        assert_eq!(deleted[0].to, "refs/heads/feature/login");
    }
}
