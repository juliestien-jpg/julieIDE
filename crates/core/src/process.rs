//! Lancement de processus externes (git, php) : sortie capturée en bloc ou en
//! flux, annulation, pas de fenêtre console sous Windows.
//!
//! Tous les appels sont **bloquants** : ils se font dans une tâche de fond
//! ([`crate::task::Tasks::spawn`]), jamais dans le fil de l'interface.
//!
//! Le terminal intégré n'utilise pas ce module : il a besoin d'une pseudo-console.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::task::CancelToken;

/// Période de consultation du jeton d'annulation et de l'état du processus.
const POLL: Duration = Duration::from_millis(20);
/// Délai accordé aux tuyaux pour se vider une fois le processus terminé, au cas
/// où un descendant détaché les garderait ouverts.
const DRAIN_GRACE: Duration = Duration::from_millis(250);

/// De quel tuyau vient un morceau de sortie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Code de sortie d'un processus ; `None` s'il a été tué par un signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: Option<i32>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Sortie complète d'un processus lancé avec [`Command::run`]. Les octets sont
/// rendus tels quels : Git peut produire autre chose que de l'UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }

    /// Transforme un code de sortie d'échec en [`Error::ProcessFailed`]. À ne pas
    /// utiliser quand un code non nul est une réponse normale (`git diff --quiet`).
    pub fn into_success(self, command: &Command) -> Result<Self> {
        if self.status.success() {
            Ok(self)
        } else {
            Err(Error::ProcessFailed {
                program: command.program_name(),
                code: self.status.code,
                stderr: self.stderr_text().trim_end().to_owned(),
            })
        }
    }
}

/// Description d'un processus à lancer. Réutilisable : chaque `run*` le relance.
///
/// Par défaut : environnement hérité, dossier courant hérité, entrée standard
/// fermée (un programme qui attendrait une saisie reçoit une fin de fichier au lieu
/// de bloquer).
#[derive(Debug, Clone)]
pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
    env_clear: bool,
    env: Vec<(OsString, Option<OsString>)>,
    stdin: Option<Vec<u8>>,
}

impl Command {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            current_dir: None,
            env_clear: false,
            env: Vec::new(),
            stdin: None,
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    pub fn current_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.current_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Définit ou remplace une variable d'environnement.
    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.env
            .push((key.as_ref().to_owned(), Some(value.as_ref().to_owned())));
        self
    }

    /// Retire une variable de l'environnement hérité.
    pub fn env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.env.push((key.as_ref().to_owned(), None));
        self
    }

    /// Part d'un environnement vide au lieu d'hériter de celui de l'IDE.
    pub fn env_clear(mut self) -> Self {
        self.env_clear = true;
        self
    }

    /// Octets à envoyer sur l'entrée standard, qui est ensuite fermée
    /// (`git commit -F -`, `git apply`…).
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    /// Nom du programme, pour les messages d'erreur.
    pub fn program_name(&self) -> String {
        self.program.to_string_lossy().into_owned()
    }

    /// Lance le processus et rend toute sa sortie à la fin.
    pub fn run(&self, cancel: &CancelToken) -> Result<Output> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = self.run_chunks(cancel, |stream, chunk| match stream {
            Stream::Stdout => stdout.extend_from_slice(chunk),
            Stream::Stderr => stderr.extend_from_slice(chunk),
        })?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    /// Lance le processus et appelle `on_line` pour chaque ligne, dès qu'elle est
    /// complète, sans sa fin de ligne (`\n` ou `\r\n`). Une dernière ligne sans fin
    /// de ligne est rendue à la fermeture du tuyau. Les lignes de stdout et de
    /// stderr s'entrelacent dans l'ordre d'arrivée.
    pub fn run_lines(
        &self,
        cancel: &CancelToken,
        mut on_line: impl FnMut(Stream, &[u8]),
    ) -> Result<ExitStatus> {
        self.run_split(cancel, b'\n', |stream, record| {
            on_line(stream, record.strip_suffix(b"\r").unwrap_or(record))
        })
    }

    /// Comme [`Command::run_lines`], avec un séparateur d'enregistrements au choix
    /// (0 pour les sorties `-z` de Git) et sans retirer les `\r`.
    pub fn run_split(
        &self,
        cancel: &CancelToken,
        separator: u8,
        mut on_record: impl FnMut(Stream, &[u8]),
    ) -> Result<ExitStatus> {
        let mut out = Splitter::new(separator);
        let mut err = Splitter::new(separator);
        let status = self.run_chunks(cancel, |stream, chunk| match stream {
            Stream::Stdout => out.feed(chunk, |record| on_record(stream, record)),
            Stream::Stderr => err.feed(chunk, |record| on_record(stream, record)),
        })?;
        out.finish(|record| on_record(Stream::Stdout, record));
        err.finish(|record| on_record(Stream::Stderr, record));
        Ok(status)
    }

    /// Lance le processus et appelle `on_chunk` pour chaque morceau de sortie lu,
    /// sans découpage. Si `on_chunk` est lent, le processus est ralenti (les tuyaux
    /// se remplissent) plutôt que la mémoire de l'IDE.
    ///
    /// À l'annulation, le processus **et ses descendants** sont tués et l'appel rend
    /// [`Error::Cancelled`].
    pub fn run_chunks(
        &self,
        cancel: &CancelToken,
        mut on_chunk: impl FnMut(Stream, &[u8]),
    ) -> Result<ExitStatus> {
        cancel.check()?;

        let mut child = self.build().spawn().map_err(|source| Error::ProcessSpawn {
            program: self.program_name(),
            source,
        })?;
        let job = job::Job::attach(&child);

        let (sender, receiver) = sync_channel::<(Stream, Vec<u8>)>(64);
        if let Some(pipe) = child.stdout.take() {
            let sender = sender.clone();
            std::thread::spawn(move || pump(pipe, Stream::Stdout, sender));
        }
        if let Some(pipe) = child.stderr.take() {
            let sender = sender.clone();
            std::thread::spawn(move || pump(pipe, Stream::Stderr, sender));
        }
        drop(sender);

        if let (Some(mut pipe), Some(bytes)) = (child.stdin.take(), self.stdin.clone()) {
            // Dans son propre fil : le processus peut ne lire son entrée qu'après
            // avoir écrit, et l'on se bloquerait l'un l'autre.
            std::thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            });
        }

        let mut exited_at: Option<Instant> = None;
        let mut last_poll = Instant::now();
        loop {
            if cancel.is_cancelled() {
                kill(&mut child, &job);
                return Err(Error::Cancelled);
            }
            match receiver.recv_timeout(POLL) {
                Ok((stream, chunk)) => on_chunk(stream, &chunk),
                Err(RecvTimeoutError::Timeout) => {}
                // Les deux tuyaux sont fermés : il ne reste qu'à attendre la fin.
                Err(RecvTimeoutError::Disconnected) => break,
            }
            match exited_at {
                Some(at) if at.elapsed() >= DRAIN_GRACE => break,
                Some(_) => {}
                None if last_poll.elapsed() >= POLL => {
                    last_poll = Instant::now();
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        exited_at = Some(Instant::now());
                    }
                }
                None => {}
            }
        }

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Ok(ExitStatus {
                        code: status.code(),
                    })
                }
                Ok(None) => {}
                Err(source) => {
                    kill(&mut child, &job);
                    return Err(Error::ProcessSpawn {
                        program: self.program_name(),
                        source,
                    });
                }
            }
            if cancel.is_cancelled() {
                kill(&mut child, &job);
                return Err(Error::Cancelled);
            }
            std::thread::sleep(POLL);
        }
    }

    fn build(&self) -> std::process::Command {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args);
        if let Some(dir) = &self.current_dir {
            command.current_dir(dir);
        }
        if self.env_clear {
            command.env_clear();
        }
        for (key, value) in &self.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        command
            .stdin(if self.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            /// Pas de fenêtre console qui clignote pour un programme en mode console.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }
}

/// Recolle les enregistrements coupés par les frontières de lecture.
struct Splitter {
    separator: u8,
    pending: Vec<u8>,
}

impl Splitter {
    fn new(separator: u8) -> Self {
        Self {
            separator,
            pending: Vec::new(),
        }
    }

    fn feed(&mut self, chunk: &[u8], mut on_record: impl FnMut(&[u8])) {
        let mut rest = chunk;
        while let Some(at) = rest.iter().position(|&byte| byte == self.separator) {
            if self.pending.is_empty() {
                on_record(&rest[..at]);
            } else {
                self.pending.extend_from_slice(&rest[..at]);
                on_record(&self.pending);
                self.pending.clear();
            }
            rest = &rest[at + 1..];
        }
        self.pending.extend_from_slice(rest);
    }

    /// Rend le dernier enregistrement s'il n'était pas terminé par le séparateur.
    fn finish(self, mut on_record: impl FnMut(&[u8])) {
        if !self.pending.is_empty() {
            on_record(&self.pending);
        }
    }
}

fn pump(
    mut pipe: impl Read,
    stream: Stream,
    sender: std::sync::mpsc::SyncSender<(Stream, Vec<u8>)>,
) {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if sender.send((stream, buffer[..read].to_vec())).is_err() {
                    break;
                }
            }
        }
    }
}

fn kill(child: &mut Child, job: &job::Job) {
    job.terminate();
    let _ = child.kill();
    let _ = child.wait();
}

/// Sous Windows, tuer un processus ne tue pas ses enfants (`git` lance des
/// sous-commandes et des crochets, `php` peut lancer des processus isolés). On
/// place donc le processus dans un objet « job », que l'on termine d'un coup à
/// l'annulation. Le job n'est **pas** réglé pour tuer à la fermeture : un
/// descendant légitimement détaché (`git gc --auto`) survit à une fin normale.
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject,
    };

    pub struct Job(HANDLE);

    impl Job {
        /// Meilleur effort : si le job ne peut être créé ou attaché, l'annulation
        /// ne tuera que le processus lui-même.
        pub fn attach(child: &Child) -> Job {
            // SAFETY : appels Win32 sans pointeur à nous ; les poignées sont
            // valides (celle du job vient d'être créée, celle du processus
            // appartient à `child`, vivant pendant l'appel).
            unsafe {
                let handle = CreateJobObjectW(null(), null());
                if !handle.is_null() {
                    AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE);
                }
                Job(handle)
            }
        }

        pub fn terminate(&self) {
            if !self.0.is_null() {
                // SAFETY : la poignée est valide jusqu'au `drop`.
                unsafe {
                    TerminateJobObject(self.0, 1);
                }
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY : poignée créée par nous, fermée une seule fois.
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod job {
    use std::process::Child;

    pub struct Job;

    impl Job {
        pub fn attach(_child: &Child) -> Job {
            Job
        }

        pub fn terminate(&self) {}
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn cmd(script: &str) -> Command {
        Command::new("cmd").arg("/C").arg(script)
    }

    /// Une pause d'environ `seconds` secondes, sans dépendre du PATH.
    const fn pause(seconds: u32) -> PauseScript {
        PauseScript(seconds)
    }

    struct PauseScript(u32);

    impl std::fmt::Display for PauseScript {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                r"%SystemRoot%\System32\ping.exe -n {} 127.0.0.1",
                self.0 + 1
            )
        }
    }

    #[test]
    fn run_captures_stdout_stderr_and_exit_code() {
        let output = cmd("echo sortie& echo erreur 1>&2& exit 3")
            .run(&CancelToken::new())
            .unwrap();
        assert_eq!(output.status.code, Some(3));
        assert!(!output.status.success());
        assert_eq!(output.stdout_text().trim(), "sortie");
        assert_eq!(output.stderr_text().trim(), "erreur");
    }

    #[test]
    fn into_success_turns_a_failure_code_into_an_error() {
        let command = cmd("echo cassé 1>&2& exit 2");
        let error = command
            .run(&CancelToken::new())
            .unwrap()
            .into_success(&command)
            .unwrap_err();
        assert!(matches!(
            error,
            Error::ProcessFailed { code: Some(2), ref program, .. } if program == "cmd"
        ));
        assert!(cmd("exit 0")
            .run(&CancelToken::new())
            .unwrap()
            .into_success(&command)
            .is_ok());
    }

    #[test]
    fn run_lines_gives_each_line_without_its_ending() {
        let mut lines = Vec::new();
        let status = cmd("echo un& echo deux& echo trois")
            .run_lines(&CancelToken::new(), |stream, line| {
                lines.push((stream, String::from_utf8_lossy(line).into_owned()));
            })
            .unwrap();
        assert!(status.success());
        assert_eq!(
            lines,
            vec![
                (Stream::Stdout, "un".to_owned()),
                (Stream::Stdout, "deux".to_owned()),
                (Stream::Stdout, "trois".to_owned()),
            ]
        );
    }

    #[test]
    fn lines_arrive_while_the_process_is_still_running() {
        let started = Instant::now();
        let mut first_line_after = None;
        cmd(&format!("echo tot& {} >nul& echo tard", pause(2)))
            .run_lines(&CancelToken::new(), |_, _| {
                first_line_after.get_or_insert(started.elapsed());
            })
            .unwrap();
        let total = started.elapsed();
        let first = first_line_after.expect("au moins une ligne");
        assert!(total >= Duration::from_millis(1500), "total {total:?}");
        assert!(
            first < Duration::from_millis(1000),
            "première ligne à {first:?}"
        );
    }

    #[test]
    fn environment_directory_and_stdin_are_passed() {
        let dir = tempfile::tempdir().unwrap();
        let output = cmd("echo %JULIE_ESSAI%& cd")
            .env("JULIE_ESSAI", "regle")
            .current_dir(dir.path())
            .run(&CancelToken::new())
            .unwrap();
        let text = output.stdout_text();
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("regle"));
        let reported = std::path::PathBuf::from(lines.next().unwrap().trim());
        assert_eq!(
            std::fs::canonicalize(reported).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );

        let output = Command::new("sort")
            .stdin("b\r\na\r\n")
            .run(&CancelToken::new())
            .unwrap();
        assert_eq!(output.stdout_text(), "a\r\nb\r\n");
    }

    #[test]
    fn a_missing_program_is_a_spawn_error() {
        let error = Command::new("julie-programme-introuvable")
            .run(&CancelToken::new())
            .unwrap_err();
        assert!(matches!(error, Error::ProcessSpawn { .. }));
    }

    #[test]
    fn cancelling_kills_the_process_and_its_children_quickly() {
        let cancel = CancelToken::new();
        let canceller = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            canceller.cancel();
        });
        let started = Instant::now();
        // `cmd` lance `ping` : c'est le petit-enfant qui tient les tuyaux.
        let error = cmd(&pause(30).to_string()).run(&cancel).unwrap_err();
        assert!(error.is_cancelled());
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "annulation en {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn cancelling_kills_grandchildren_too() {
        let dir = tempfile::tempdir().unwrap();
        // Le `cmd` intérieur est un petit-enfant : s'il survivait à l'annulation,
        // il créerait le fichier témoin au bout de deux secondes.
        let marker = dir.path().join("temoin.txt");
        let lines = [
            r"@%SystemRoot%\System32\ping.exe -n %1 127.0.0.1 >nul",
            "@echo x > temoin.txt",
            "",
        ];
        std::fs::write(dir.path().join("lent.bat"), lines.join("\r\n")).unwrap();
        let nested = |pings: &str| {
            Command::new("cmd")
                .args(["/C", "cmd", "/C", r".\lent.bat", pings])
                .current_dir(dir.path())
        };

        // Témoin du témoin : sans annulation, le fichier est bien créé.
        let control = nested("1").run(&CancelToken::new()).unwrap();
        assert!(marker.exists(), "{control:?}");
        std::fs::remove_file(&marker).unwrap();

        let cancel = CancelToken::new();
        let canceller = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            canceller.cancel();
        });
        let error = nested("3").run(&cancel).unwrap_err();
        assert!(error.is_cancelled());
        std::thread::sleep(Duration::from_millis(3000));
        assert!(!marker.exists());
    }

    #[test]
    fn an_already_cancelled_token_starts_nothing() {
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(cmd("echo x").run(&cancel).unwrap_err().is_cancelled());
    }

    #[test]
    fn splitter_rebuilds_records_cut_anywhere() {
        let input = b"un\0deux\0\0trois";
        for size in 1..=input.len() {
            let mut records: Vec<Vec<u8>> = Vec::new();
            let mut splitter = Splitter::new(0);
            for chunk in input.chunks(size) {
                splitter.feed(chunk, |record| records.push(record.to_vec()));
            }
            splitter.finish(|record| records.push(record.to_vec()));
            assert_eq!(
                records,
                vec![
                    b"un".to_vec(),
                    b"deux".to_vec(),
                    b"".to_vec(),
                    b"trois".to_vec()
                ],
                "morceaux de {size} octets"
            );
        }
    }

    #[test]
    fn a_large_output_is_captured_whole() {
        // Bien plus que la capacité d'un tuyau : vérifie qu'on ne se bloque pas
        // et qu'on ne perd rien.
        let mut input = Vec::new();
        for index in 0..100_000u32 {
            input.extend_from_slice(format!("ligne-{index:06}\r\n").as_bytes());
        }
        let mut count = 0u32;
        let mut in_order = true;
        let status = Command::new("sort")
            .stdin(input)
            .run_lines(&CancelToken::new(), |_, line| {
                in_order &= line == format!("ligne-{count:06}").as_bytes();
                count += 1;
            })
            .unwrap();
        assert!(status.success());
        assert!(in_order);
        assert_eq!(count, 100_000);
    }
}
