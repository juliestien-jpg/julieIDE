//! Le point d'entrée unique vers l'exécutable `git`, et le journal des commandes
//! lancées (onglet Console).
//!
//! Toute commande Git de la crate passe par [`Git::run`] : environnement neutre,
//! options globales fixes, annulation, inscription au journal.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use julie_core::process::{Command, Output, Stream};
use julie_core::{CancelToken, Domain, Publisher};

use crate::error::{Result, VcsError};
use crate::service::VcsEvent;

/// Section `[vcs]` des réglages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct VcsSettings {
    /// Chemin de l'exécutable `git` ; `None` : `git` trouvé dans le PATH.
    pub git_path: Option<PathBuf>,
    /// Branches où force push et suppression distante sont refusés. Noms
    /// exacts, ou motif terminé par `*` (`release/*`). Défaut : `main`, `master`.
    pub protected_branches: Vec<String>,
    /// Méthode de « Mettre à jour le projet » mémorisée (`"fusion"` ou
    /// `"rebase"`) ; `None` : demander.
    pub update_method: Option<IntegrationMethod>,
}

impl Default for VcsSettings {
    fn default() -> Self {
        Self {
            git_path: None,
            protected_branches: vec!["main".to_owned(), "master".to_owned()],
            update_method: None,
        }
    }
}

impl VcsSettings {
    /// La branche (nom court, sans le distant) est-elle protégée ?
    pub fn is_protected(&self, branch: &str) -> bool {
        is_protected(&self.protected_branches, branch)
    }
}

/// Une branche est-elle dans la liste (noms exacts ou motifs `préfixe*`) ?
pub fn is_protected(protected: &[String], branch: &str) -> bool {
    protected
        .iter()
        .any(|pattern| match pattern.strip_suffix('*') {
            Some(prefix) => branch.starts_with(prefix),
            None => pattern == branch,
        })
}

/// Intégrer les changements de l'amont : fusion ou rebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Deserialize)]
pub enum IntegrationMethod {
    #[serde(rename = "fusion")]
    Merge,
    #[serde(rename = "rebase")]
    Rebase,
}

/// Nombre de commandes gardées au journal.
const LOG_CAPACITY: usize = 500;
/// Taille gardée de chaque sortie au journal (le reste est tronqué).
const OUTPUT_EXCERPT: usize = 16 * 1024;

/// Options passées avant la sous-commande, à chaque appel.
const GLOBAL_OPTIONS: [&str; 7] = [
    "--no-pager",
    "-c",
    "color.ui=false",
    "-c",
    "core.quotepath=false",
    "-c",
    "log.showSignature=false",
];

/// Reçoit la sortie d'erreur au fil de l'eau.
type StderrSink<'a> = &'a mut dyn FnMut(&[u8]);

/// Lecture seule ou écriture : une lecture pose `GIT_OPTIONAL_LOCKS=0` (Git ne
/// rafraîchit pas l'index en douce et ne gêne pas une écriture concurrente).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Read,
    Write,
}

/// L'exécutable `git` et son journal. Peu coûteux à cloner ; tous les clones
/// partagent le même journal.
#[derive(Clone)]
pub struct Git {
    program: OsString,
    extra_env: Vec<(OsString, OsString)>,
    log: CommandLog,
    publisher: Option<Publisher>,
}

impl std::fmt::Debug for Git {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Git")
            .field("program", &self.program)
            .field("extra_env", &self.extra_env)
            .finish_non_exhaustive()
    }
}

impl Git {
    pub fn new(settings: &VcsSettings) -> Self {
        let program = settings
            .git_path
            .as_ref()
            .map(|path| path.as_os_str().to_owned())
            .unwrap_or_else(|| OsString::from("git"));
        Self {
            program,
            extra_env: Vec::new(),
            log: CommandLog::default(),
            publisher: None,
        }
    }

    /// Ajoute une variable d'environnement à chaque appel, posée après
    /// l'environnement neutre. Sert surtout aux tests (`GIT_CONFIG_GLOBAL`,
    /// `GIT_CONFIG_NOSYSTEM`) pour isoler la configuration de la machine.
    pub fn with_env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.extra_env
            .push((key.as_ref().to_owned(), value.as_ref().to_owned()));
        self
    }

    /// Chaque commande lancée sera aussi publiée sur le bus
    /// ([`VcsEvent::Command`], au début puis à la fin).
    pub fn with_publisher(mut self, publisher: Publisher) -> Self {
        self.publisher = Some(publisher);
        self
    }

    /// Le même Git sans `GIT_LITERAL_PATHSPECS` : pour les commandes qui ne
    /// reçoivent aucun chemin de nous mais en fabriquent en interne
    /// (`git stash push --include-untracked` ne nettoie plus les non suivis
    /// sous chemins littéraux : ils resteraient sur le disque).
    pub(crate) fn without_literal_pathspecs(&self) -> Git {
        self.clone().with_env("GIT_LITERAL_PATHSPECS", "0")
    }

    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Le journal des commandes lancées.
    pub fn log(&self) -> &CommandLog {
        &self.log
    }

    /// Lance `git <args>` dans `cwd`. **Bloquant** (tâche de fond). Un code de
    /// sortie non nul n'est pas une erreur ici : voir [`Git::run_ok`].
    pub(crate) fn run(
        &self,
        cwd: &Path,
        args: &[OsString],
        access: Access,
        stdin: Option<Vec<u8>>,
        cancel: &CancelToken,
    ) -> Result<Output> {
        self.run_inner(cwd, args, access, stdin, cancel, None)
    }

    /// Comme [`Git::run`], en passant chaque morceau de la sortie d'erreur à
    /// `on_stderr` dès qu'il arrive (progression d'un fetch, d'un push…).
    pub(crate) fn run_streaming(
        &self,
        cwd: &Path,
        args: &[OsString],
        cancel: &CancelToken,
        on_stderr: &mut dyn FnMut(&[u8]),
    ) -> Result<Output> {
        self.run_inner(cwd, args, Access::Write, None, cancel, Some(on_stderr))
    }

    fn run_inner(
        &self,
        cwd: &Path,
        args: &[OsString],
        access: Access,
        stdin: Option<Vec<u8>>,
        cancel: &CancelToken,
        mut on_stderr: Option<StderrSink<'_>>,
    ) -> Result<Output> {
        let mut command = Command::new(&self.program)
            .args(GLOBAL_OPTIONS)
            .args(args)
            .current_dir(cwd)
            // Messages en anglais, stables : on en reconnaît certains (verrou…).
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env_remove("LANGUAGE")
            // Jamais de saisie : ni identifiants, ni éditeur.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", ":")
            .env("GIT_SEQUENCE_EDITOR", ":")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_PAGER", "cat")
            // Un chemin est un chemin, pas un motif (`[`, `*` dans un nom).
            .env("GIT_LITERAL_PATHSPECS", "1")
            // Un environnement hérité d'un crochet ne doit pas viser un autre dépôt.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_PREFIX");
        command = match access {
            Access::Read => command.env("GIT_OPTIONAL_LOCKS", "0"),
            Access::Write => command.env_remove("GIT_OPTIONAL_LOCKS"),
        };
        for (key, value) in &self.extra_env {
            command = command.env(key, value);
        }
        if let Some(bytes) = stdin {
            command = command.stdin(bytes);
        }

        let shown: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let record = self.log.start(shown, cwd.to_path_buf());
        self.announce(&record);

        let started = Instant::now();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = command
            .run_chunks(cancel, |stream, chunk| match stream {
                Stream::Stdout => stdout.extend_from_slice(chunk),
                Stream::Stderr => {
                    stderr.extend_from_slice(chunk);
                    if let Some(callback) = on_stderr.as_mut() {
                        callback(chunk);
                    }
                }
            })
            .map(|status| Output {
                status,
                stdout,
                stderr,
            });
        let duration = started.elapsed();
        let (outcome, exit_code, stdout, stderr) = match &result {
            Ok(output) => (
                CommandOutcome::Exited,
                output.status.code,
                excerpt(&output.stdout),
                excerpt(&output.stderr),
            ),
            Err(error) if error.is_cancelled() => (
                CommandOutcome::Cancelled,
                None,
                String::new(),
                String::new(),
            ),
            Err(error) => (
                CommandOutcome::NotStarted(error.to_string()),
                None,
                String::new(),
                String::new(),
            ),
        };
        if let Some(record) = self.log.finish(
            record.id,
            CommandResult {
                duration,
                exit_code,
                outcome,
                stdout,
                stderr,
            },
        ) {
            self.announce(&record);
        }

        match result {
            Ok(output) => Ok(output),
            Err(julie_core::Error::ProcessSpawn { program, source })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Err(VcsError::GitNotFound { program })
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Comme [`Git::run`], mais un code non nul devient une erreur
    /// ([`VcsError::GitFailed`], [`VcsError::IndexLocked`]…).
    pub(crate) fn run_ok(
        &self,
        cwd: &Path,
        args: &[OsString],
        access: Access,
        stdin: Option<Vec<u8>>,
        cancel: &CancelToken,
    ) -> Result<Output> {
        let output = self.run(cwd, args, access, stdin, cancel)?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(failure(args, &output))
        }
    }

    /// Lecture : sortie standard d'une commande qui doit réussir.
    pub(crate) fn read(
        &self,
        cwd: &Path,
        args: &[OsString],
        cancel: &CancelToken,
    ) -> Result<String> {
        Ok(self
            .run_ok(cwd, args, Access::Read, None, cancel)?
            .stdout_text())
    }

    /// `git rev-parse -q --verify <rév>^{commit}` : le commit, ou `None`.
    pub(crate) fn resolve_commit(
        &self,
        cwd: &Path,
        revision: &str,
        cancel: &CancelToken,
    ) -> Result<Option<String>> {
        let output = self.run(
            cwd,
            &args([
                "rev-parse".to_owned(),
                "-q".to_owned(),
                "--verify".to_owned(),
                "--end-of-options".to_owned(),
                format!("{revision}^{{commit}}"),
            ]),
            Access::Read,
            None,
            cancel,
        )?;
        Ok(output
            .status
            .success()
            .then(|| output.stdout_text().trim().to_owned())
            .filter(|id| !id.is_empty()))
    }

    /// `git rev-list --count <args…>`.
    pub(crate) fn count(&self, cwd: &Path, list: &[&str], cancel: &CancelToken) -> Result<u32> {
        let mut command = args(["rev-list", "--count"]);
        command.extend(list.iter().map(OsString::from));
        let text = self.read(cwd, &command, cancel)?;
        text.trim()
            .parse()
            .map_err(|_| VcsError::UnexpectedOutput(format!("rev-list --count : {text:?}")))
    }

    fn announce(&self, record: &CommandRecord) {
        if let Some(publisher) = &self.publisher {
            publisher.publish_domain(Domain::Vcs, VcsEvent::Command(record.clone()));
        }
    }
}

/// Erreur à partir de la sortie d'une commande en échec.
pub(crate) fn failure(args: &[OsString], output: &Output) -> VcsError {
    let stderr = output.stderr_text();
    let stdout = output.stdout_text();
    if stderr.contains("index.lock") && stderr.contains("File exists") {
        return VcsError::IndexLocked(stderr.trim_end().to_owned());
    }
    if stderr.contains("dubious ownership") {
        return VcsError::UnsafeRepository(stderr.trim_end().to_owned());
    }
    if is_authentication_failure(&stderr) {
        return VcsError::AuthenticationFailed {
            stderr: stderr.trim_end().to_owned(),
        };
    }
    VcsError::GitFailed {
        command: args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
        code: output.status.code,
        stdout,
        stderr,
    }
}

/// La sortie d'erreur dit-elle que Git voulait des identifiants (ou qu'on les
/// a refusés) ? Messages anglais (`LC_ALL=C`) de Git, du gestionnaire
/// d'identifiants, de SSH et des forges courantes. « Could not read from remote
/// repository » seul ne suffit pas : Git le dit aussi d'un chemin inexistant.
pub fn is_authentication_failure(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    [
        "terminal prompts disabled",
        "could not read username",
        "could not read password",
        "authentication failed",
        "permission denied (publickey",
        "permission denied, please try again",
        "host key verification failed",
        "invalid username or password",
        "http basic: access denied",
        "the requested url returned error: 401",
        "the requested url returned error: 403",
        "unable to get password from user",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Construit une liste d'arguments à partir de chaînes.
pub(crate) fn args<I, S>(list: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    list.into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect()
}

fn excerpt(bytes: &[u8]) -> String {
    if bytes.len() <= OUTPUT_EXCERPT {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        let mut text = String::from_utf8_lossy(&bytes[..OUTPUT_EXCERPT]).into_owned();
        text.push_str("\n[…]");
        text
    }
}

/// Une commande Git lancée par l'IDE, pour la Console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRecord {
    /// Croissant ; le même identifiant revient au début puis à la fin.
    pub id: u64,
    /// Arguments après `git` (sans les options globales fixes).
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub started_at: SystemTime,
    /// `None` tant que la commande tourne.
    pub result: Option<CommandResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub duration: Duration,
    /// Code de sortie ; `None` si annulée ou non lancée.
    pub exit_code: Option<i32>,
    pub outcome: CommandOutcome,
    /// Début de la sortie standard (16 Kio au plus, puis « […] »).
    pub stdout: String,
    /// Début de la sortie d'erreur (16 Kio au plus).
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Le processus s'est terminé (voir `exit_code`).
    Exited,
    Cancelled,
    /// Lancement impossible (message technique).
    NotStarted(String),
}

/// Journal borné (les 500 dernières commandes). Clonable, partagé.
#[derive(Clone, Default)]
pub struct CommandLog {
    inner: Arc<Mutex<LogState>>,
}

#[derive(Default)]
struct LogState {
    next_id: u64,
    entries: VecDeque<CommandRecord>,
}

impl CommandLog {
    /// Copie des entrées, de la plus ancienne à la plus récente. Ne touche ni au
    /// disque ni à Git : utilisable depuis l'interface.
    pub fn entries(&self) -> Vec<CommandRecord> {
        self.lock().entries.iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.lock().entries.clear();
    }

    fn start(&self, args: Vec<String>, cwd: PathBuf) -> CommandRecord {
        let mut state = self.lock();
        state.next_id += 1;
        let record = CommandRecord {
            id: state.next_id,
            args,
            cwd,
            started_at: SystemTime::now(),
            result: None,
        };
        if state.entries.len() == LOG_CAPACITY {
            state.entries.pop_front();
        }
        state.entries.push_back(record.clone());
        record
    }

    fn finish(&self, id: u64, result: CommandResult) -> Option<CommandRecord> {
        let mut state = self.lock();
        let entry = state
            .entries
            .iter_mut()
            .rev()
            .find(|entry| entry.id == id)?;
        entry.result = Some(result);
        Some(entry.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LogState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_bounded_and_keeps_the_latest() {
        let log = CommandLog::default();
        for index in 0..(LOG_CAPACITY + 10) {
            let record = log.start(vec![format!("c{index}")], PathBuf::from("x"));
            assert!(log
                .finish(
                    record.id,
                    CommandResult {
                        duration: Duration::ZERO,
                        exit_code: Some(0),
                        outcome: CommandOutcome::Exited,
                        stdout: String::new(),
                        stderr: String::new(),
                    }
                )
                .is_some());
        }
        let entries = log.entries();
        assert_eq!(entries.len(), LOG_CAPACITY);
        assert_eq!(
            entries.last().unwrap().args,
            vec![format!("c{}", LOG_CAPACITY + 9)]
        );
        assert!(entries.iter().all(|entry| entry.result.is_some()));
    }

    #[test]
    fn long_outputs_are_truncated() {
        let text = excerpt(&vec![b'a'; OUTPUT_EXCERPT + 5]);
        assert!(text.ends_with("[…]"));
        assert_eq!(excerpt(b"court"), "court");
    }
}
