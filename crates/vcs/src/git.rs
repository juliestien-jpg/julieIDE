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

use julie_core::process::{Command, Output};
use julie_core::{CancelToken, Domain, Publisher};

use crate::error::{Result, VcsError};
use crate::service::VcsEvent;

/// Section `[vcs]` des réglages.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct VcsSettings {
    /// Chemin de l'exécutable `git` ; `None` : `git` trouvé dans le PATH.
    pub git_path: Option<PathBuf>,
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
        let result = command.run(cancel);
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
