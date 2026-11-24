//! Sessions : un interpréteur dans une pseudo-console, son émulateur, ses fils.
//!
//! Par session, deux fils :
//! - **lecture** (une tâche `terminal.session` du noyau) : lance l'interpréteur,
//!   lit la sortie, l'analyse dans l'émulateur et publie `Output` ;
//! - **contrôle** : écrit la saisie, applique les redimensionnements, surveille
//!   la fin de l'interpréteur et ferme la pseudo-console.
//!
//! Le fil de l'interface ne fait que poser des commandes dans un canal et
//! copier l'écran sous verrou (quelques microsecondes).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use julie_core::bus::Publisher;
use julie_core::task::{TaskContext, TaskHandle, Tasks};
use julie_core::Domain;

use crate::emulator::{Emulator, ScreenPoint, ScrollAction, SelectionKind, Signal};
use crate::error::TerminalError;
use crate::input::{encode_key, encode_paste, Key, Modifiers};
use crate::profile::{detect_profiles, Profile, Profiles, TerminalSettings};
use crate::pty::{process_snapshot, Pty, SpawnSpec};
use crate::screen::Screen;

/// Identifiant d'une session, unique dans le processus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(pub u64);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "terminal-{}", self.0)
    }
}

/// État d'une session.
#[derive(Debug, Clone)]
pub enum SessionStatus {
    /// L'interpréteur est en cours de lancement.
    Starting,
    Running {
        pid: u32,
    },
    /// L'interpréteur s'est terminé de lui-même (`exit`, `exit 3`…).
    Exited {
        code: u32,
    },
    /// Fermée à la demande (`Session::close`) : arbre de processus tué.
    Closed,
    /// N'a pas pu démarrer (interpréteur introuvable…).
    Failed(Arc<TerminalError>),
}

impl SessionStatus {
    /// Plus rien ne tourne.
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            SessionStatus::Exited { .. } | SessionStatus::Closed | SessionStatus::Failed(_)
        )
    }
}

/// Événements publiés sous `Domain::Terminal`
/// (`event.get::<julie_terminal::TerminalEvent>()`).
#[derive(Debug)]
pub enum TerminalEvent {
    /// Résultat de `Terminals::detect_profiles`.
    Profiles(Profiles),
    /// L'interpréteur tourne.
    Started {
        session: SessionId,
        pid: u32,
    },
    /// L'écran a changé : le redessiner (`Session::snapshot`). **Coalescé** : un
    /// seul `Output` en attente par session tant que l'écran n'a pas été relu.
    Output {
        session: SessionId,
    },
    /// Titre fixé par le programme (OSC 0/2) ; `None` : titre par défaut.
    TitleChanged {
        session: SessionId,
        title: Option<String>,
    },
    Bell {
        session: SessionId,
    },
    /// Le programme demande à copier ce texte (OSC 52).
    Clipboard {
        session: SessionId,
        text: String,
    },
    /// L'interpréteur s'est terminé de lui-même ; tout l'arbre est arrêté.
    Exited {
        session: SessionId,
        code: u32,
    },
    /// Fermée à la demande ; tout l'arbre est arrêté.
    Closed {
        session: SessionId,
    },
    /// N'a pas pu démarrer.
    Failed {
        session: SessionId,
        error: Arc<TerminalError>,
    },
}

/// Ce qu'il faut pour ouvrir une session.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    pub profile: Profile,
    /// Dossier de départ (racine du projet).
    pub cwd: PathBuf,
    /// Taille initiale en cellules (la vue la corrige dès sa première image).
    pub columns: u16,
    pub lines: u16,
}

impl SessionOptions {
    pub fn new(profile: Profile, cwd: impl Into<PathBuf>) -> Self {
        Self {
            profile,
            cwd: cwd.into(),
            columns: 80,
            lines: 24,
        }
    }
}

/// Le service : construit par `app`, ouvre les sessions.
#[derive(Clone)]
pub struct Terminals {
    tasks: Tasks,
    settings: TerminalSettings,
    next_id: Arc<AtomicU64>,
}

impl Terminals {
    pub fn new(tasks: Tasks, settings: TerminalSettings) -> Self {
        Self {
            tasks,
            settings,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn settings(&self) -> &TerminalSettings {
        &self.settings
    }

    /// Détecte les interpréteurs (accès disque) ; publie `TerminalEvent::Profiles`.
    /// Clé de tâche `terminal.profiles`.
    pub fn detect_profiles(&self) -> TaskHandle {
        let settings = self.settings.clone();
        self.tasks
            .spawn(Domain::Terminal, "terminal.profiles", move |ctx| {
                ctx.publish(TerminalEvent::Profiles(detect_profiles(&settings)));
                Ok(())
            })
    }

    /// Ouvre une session ; rend la main tout de suite (le lancement se fait
    /// dans la tâche `terminal.session`). La saisie envoyée avant le démarrage
    /// est gardée et transmise dès que l'interpréteur tourne.
    pub fn open(&self, options: SessionOptions) -> Session {
        let id = SessionId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (commands, receiver) = mpsc::channel();
        let shared = Arc::new(Shared {
            id,
            profile: options.profile.clone(),
            emulator: Mutex::new(Emulator::new(
                options.columns as usize,
                options.lines as usize,
                self.settings.scrollback(),
            )),
            status: Mutex::new(SessionStatus::Starting),
            title: Mutex::new(None),
            generation: AtomicU64::new(1),
            notified: AtomicBool::new(false),
            closing: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            pty: OnceLock::new(),
            last_output: Mutex::new(Instant::now()),
            size: Mutex::new((options.columns.max(2), options.lines.max(1))),
        });
        let spec = SpawnSpec {
            program: options.profile.program.clone(),
            args: options.profile.args.iter().map(Into::into).collect(),
            cwd: options.cwd,
            env: options
                .profile
                .env
                .iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            columns: options.columns.max(2),
            lines: options.lines.max(1),
        };
        let task = {
            let shared = shared.clone();
            let commands = commands.clone();
            self.tasks
                .spawn(Domain::Terminal, "terminal.session", move |ctx| {
                    run_session(ctx, shared, spec, receiver, commands);
                    Ok(())
                })
        };
        Session {
            shared,
            commands,
            task,
        }
    }
}

enum Command {
    Write(Vec<u8>),
    Resize {
        columns: u16,
        lines: u16,
        cell_width: u16,
        cell_height: u16,
    },
    Close,
}

struct Shared {
    id: SessionId,
    profile: Profile,
    emulator: Mutex<Emulator>,
    status: Mutex<SessionStatus>,
    title: Mutex<Option<String>>,
    /// Augmente à chaque changement de ce qui est affiché.
    generation: AtomicU64,
    /// Un `Output` publié n'a pas encore été suivi d'une relecture de l'écran.
    notified: AtomicBool,
    closing: AtomicBool,
    /// Le fil de lecture a fini : le fil de contrôle s'arrête.
    finished: AtomicBool,
    pty: OnceLock<Arc<Pty>>,
    last_output: Mutex<Instant>,
    /// Dernière taille demandée (cellules).
    size: Mutex<(u16, u16)>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Marque l'écran changé et publie `Output` si aucun n'est en attente.
    fn changed_and_notify(&self, publish: impl FnOnce(TerminalEvent)) {
        self.changed();
        if !self.notified.swap(true, Ordering::AcqRel) {
            publish(TerminalEvent::Output { session: self.id });
        }
    }
}

fn run_session(
    ctx: &TaskContext,
    shared: Arc<Shared>,
    spec: SpawnSpec,
    receiver: Receiver<Command>,
    commands: Sender<Command>,
) {
    let id = shared.id;
    let (pty, mut reader) = match Pty::spawn(&spec) {
        Ok(parts) => parts,
        Err(error) => {
            let error = Arc::new(error);
            *lock(&shared.status) = SessionStatus::Failed(error.clone());
            shared.finished.store(true, Ordering::Release);
            ctx.publish(TerminalEvent::Failed { session: id, error });
            return;
        }
    };
    let pty = Arc::new(pty);
    let _ = shared.pty.set(pty.clone());
    let pid = pty.pid();
    *lock(&shared.status) = SessionStatus::Running { pid };
    ctx.publish(TerminalEvent::Started { session: id, pid });

    let control = {
        let shared = shared.clone();
        let pty = pty.clone();
        let publisher = ctx.publisher().clone();
        std::thread::Builder::new()
            .name(format!("{id}-controle"))
            .spawn(move || control_loop(&shared, &pty, receiver, &publisher))
    };
    if control.is_err() {
        // Sans fil de contrôle, rien ne fermerait la console : on arrête tout.
        shared.closing.store(true, Ordering::Release);
        pty.shutdown();
    }

    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let signals = lock(&shared.emulator).feed(&buffer[..read]);
        *lock(&shared.last_output) = Instant::now();
        for signal in signals {
            match signal {
                Signal::Reply(bytes) => {
                    let _ = commands.send(Command::Write(bytes));
                }
                Signal::Title(title) => {
                    *lock(&shared.title) = title.clone();
                    ctx.publish(TerminalEvent::TitleChanged { session: id, title });
                }
                Signal::Clipboard(text) => {
                    ctx.publish(TerminalEvent::Clipboard { session: id, text });
                }
                Signal::Bell => ctx.publish(TerminalEvent::Bell { session: id }),
            }
        }
        shared.changed_and_notify(|event| ctx.publish(event));
    }

    // Fin de la sortie : la console est fermée (ou la lecture a échoué).
    pty.shutdown();
    let code = pty.wait(2_000).unwrap_or(1);
    let closed = shared.closing.load(Ordering::Acquire);
    *lock(&shared.status) = if closed {
        SessionStatus::Closed
    } else {
        SessionStatus::Exited { code }
    };
    shared.finished.store(true, Ordering::Release);
    if let Ok(control) = control {
        let _ = control.join();
    }
    shared.changed();
    ctx.publish(TerminalEvent::Output { session: id });
    ctx.publish(if closed {
        TerminalEvent::Closed { session: id }
    } else {
        TerminalEvent::Exited { session: id, code }
    });
}

/// Après la fin de l'interpréteur, on attend que la sortie se taise (conhost
/// rend ses dernières lignes un peu après) avant de fermer la console.
const QUIET_AFTER_EXIT: Duration = Duration::from_millis(150);
const MAX_WAIT_AFTER_EXIT: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_millis(50);

fn control_loop(shared: &Shared, pty: &Pty, receiver: Receiver<Command>, publisher: &Publisher) {
    let publish = |event| publisher.publish_domain(Domain::Terminal, event);
    let mut exited_at: Option<Instant> = None;
    loop {
        if shared.finished.load(Ordering::Acquire) {
            return;
        }
        match receiver.recv_timeout(TICK) {
            Ok(Command::Write(bytes)) => {
                if pty.write_all(&bytes).is_err() {
                    // Console fermée : la lecture va rendre la fin.
                }
            }
            Ok(Command::Resize {
                columns,
                lines,
                cell_width,
                cell_height,
            }) => {
                lock(&shared.emulator).resize(
                    columns as usize,
                    lines as usize,
                    cell_width,
                    cell_height,
                );
                let _ = pty.resize(columns, lines);
                shared.changed_and_notify(publish);
            }
            Ok(Command::Close) | Err(RecvTimeoutError::Disconnected) => {
                shared.closing.store(true, Ordering::Release);
                pty.shutdown();
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }

        let now = Instant::now();
        if lock(&shared.emulator).flush_sync_timeout(now) {
            shared.changed_and_notify(publish);
        }
        if exited_at.is_none() && pty.try_wait().is_some() {
            exited_at = Some(now);
        }
        if let Some(exited) = exited_at {
            let quiet = lock(&shared.last_output).elapsed() >= QUIET_AFTER_EXIT
                && now.duration_since(exited) >= QUIET_AFTER_EXIT;
            if quiet || now.duration_since(exited) >= MAX_WAIT_AFTER_EXIT {
                // L'interpréteur est fini : on arrête ce qu'il a pu laisser
                // tourner et on ferme la console ; la lecture publiera `Exited`.
                pty.shutdown();
                return;
            }
        }
    }
}

/// Une session ouverte, possédée par l'onglet qui l'affiche. La lâcher la
/// ferme (comme [`Session::close`]).
pub struct Session {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    task: TaskHandle,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.shared.id)
            .field("profile", &self.shared.profile.kind)
            .field("status", &self.status())
            .finish()
    }
}

impl Session {
    pub fn id(&self) -> SessionId {
        self.shared.id
    }

    pub fn profile(&self) -> &Profile {
        &self.shared.profile
    }

    /// Tâche de lecture (`terminal.session`).
    pub fn task(&self) -> &TaskHandle {
        &self.task
    }

    pub fn status(&self) -> SessionStatus {
        lock(&self.shared.status).clone()
    }

    /// Titre fixé par le programme, s'il en a fixé un.
    pub fn title(&self) -> Option<String> {
        lock(&self.shared.title).clone()
    }

    /// Augmente à chaque changement de l'affichage : la vue ne recopie l'écran
    /// que lorsqu'il a bougé.
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    /// Copie la partie affichée dans `screen` et réarme la publication de
    /// `Output`. Verrou tenu le temps de la copie seulement.
    pub fn snapshot(&self, screen: &mut Screen) -> u64 {
        self.shared.notified.store(false, Ordering::Release);
        let generation = self.generation();
        lock(&self.shared.emulator).snapshot(screen);
        generation
    }

    /// Octets bruts vers le programme (sans défilement ni codage).
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.commands.send(Command::Write(bytes));
        }
    }

    /// Saisie de l'utilisatrice : ramène la vue en bas de l'historique.
    fn input(&self, bytes: Vec<u8>) {
        {
            let mut emulator = lock(&self.shared.emulator);
            if emulator.display_offset() != 0 {
                emulator.scroll(ScrollAction::Bottom);
                self.shared.changed();
            }
        }
        self.write(bytes);
    }

    /// Texte tapé (lettres, accents composés, AltGr), envoyé en UTF-8.
    pub fn send_text(&self, text: &str) {
        self.input(text.as_bytes().to_vec());
    }

    /// Touche spéciale ou combinaison ; `false` si elle n'a pas de codage.
    pub fn send_key(&self, key: Key, modifiers: Modifiers) -> bool {
        let modes = lock(&self.shared.emulator).modes();
        match encode_key(key, modifiers, &modes) {
            Some(bytes) => {
                self.input(bytes);
                true
            }
            None => false,
        }
    }

    /// Colle `text` (encadré si le programme l'a demandé).
    pub fn paste(&self, text: &str) {
        let bracketed = lock(&self.shared.emulator).modes().bracketed_paste;
        self.input(encode_paste(text, bracketed));
    }

    /// Taille disponible en cellules (et taille d'une cellule en pixels). Sans
    /// effet si elle n'a pas changé ; sinon émulateur et pseudo-console suivent.
    pub fn resize(&self, columns: u16, lines: u16, cell_width: u16, cell_height: u16) {
        let columns = columns.max(2);
        let lines = lines.max(1);
        {
            let mut size = lock(&self.shared.size);
            if *size == (columns, lines) {
                return;
            }
            *size = (columns, lines);
        }
        let _ = self.commands.send(Command::Resize {
            columns,
            lines,
            cell_width,
            cell_height,
        });
    }

    /// Dernière taille demandée (cellules).
    pub fn size(&self) -> (u16, u16) {
        *lock(&self.shared.size)
    }

    /// Défile la vue dans l'historique.
    pub fn scroll(&self, action: ScrollAction) {
        lock(&self.shared.emulator).scroll(action);
        self.shared.changed();
    }

    /// Molette : `lines` positif vers le haut. Dans l'écran alternatif (less,
    /// vim), envoie des flèches au programme ; sinon défile l'historique.
    pub fn wheel(&self, lines: i32) {
        if lines == 0 {
            return;
        }
        let modes = lock(&self.shared.emulator).modes();
        if modes.alt_screen && modes.alternate_scroll && !modes.mouse {
            let key = if lines > 0 { Key::Up } else { Key::Down };
            if let Some(bytes) = encode_key(key, Modifiers::NONE, &modes) {
                self.write(bytes.repeat(lines.unsigned_abs() as usize));
            }
        } else {
            self.scroll(ScrollAction::Lines(lines));
        }
    }

    pub fn start_selection(&self, point: ScreenPoint, kind: SelectionKind) {
        lock(&self.shared.emulator).start_selection(point, kind);
        self.shared.changed();
    }

    pub fn extend_selection(&self, point: ScreenPoint) {
        lock(&self.shared.emulator).extend_selection(point);
        self.shared.changed();
    }

    pub fn clear_selection(&self) {
        let mut emulator = lock(&self.shared.emulator);
        if emulator.has_selection() {
            emulator.clear_selection();
            self.shared.changed();
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        lock(&self.shared.emulator).selection_text()
    }

    pub fn has_selection(&self) -> bool {
        lock(&self.shared.emulator).has_selection()
    }

    /// Vide l'historique de défilement (menu « Effacer »).
    pub fn clear_history(&self) {
        lock(&self.shared.emulator).clear_history();
        self.shared.changed();
    }

    /// Identifiant du processus de l'interpréteur (une fois démarré).
    pub fn pid(&self) -> Option<u32> {
        self.shared.pty.get().map(|pty| pty.pid())
    }

    /// Processus vivants de l'arbre, interpréteur compris (vide une fois fermé).
    pub fn process_ids(&self) -> Vec<u32> {
        self.shared
            .pty
            .get()
            .map(|pty| pty.process_ids())
            .unwrap_or_default()
    }

    /// Programmes lancés depuis l'interpréteur et encore vivants (noms sans
    /// `.exe`, sans doublon), pour la confirmation de fermeture. Instantané des
    /// processus du système : quelques millisecondes.
    pub fn running_programs(&self) -> Vec<String> {
        let Some(pty) = self.shared.pty.get() else {
            return Vec::new();
        };
        if self.status().is_finished() {
            return Vec::new();
        }
        let ids = pty.process_ids();
        if ids.len() <= 1 {
            return Vec::new();
        }
        let processes: Vec<_> = process_snapshot()
            .into_iter()
            .filter(|p| ids.contains(&p.pid))
            .collect();
        let root = pty.pid();
        let Some(root_name) = processes
            .iter()
            .find(|p| p.pid == root)
            .map(|p| p.name.to_ascii_lowercase())
        else {
            return Vec::new();
        };
        // Git Bash : `bin\bash.exe` est un lanceur, le vrai shell est son
        // enfant `usr\bin\bash.exe`. Ses copies (fork MSYS) ne comptent pas
        // s'il y a autre chose.
        let strip = |name: &str| {
            let lower = name.to_ascii_lowercase();
            lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
        };
        let shell = strip(&root_name);
        let mut names: Vec<String> = Vec::new();
        for process in &processes {
            if process.pid == root {
                continue;
            }
            if process.parent == root && process.name.eq_ignore_ascii_case(&root_name) {
                continue;
            }
            let name = strip(&process.name);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        if names.iter().any(|n| *n != shell) {
            names.retain(|n| *n != shell);
        }
        names
    }

    /// Ferme la session : tue l'interpréteur et tous ses descendants, puis
    /// ferme la pseudo-console. Ne bloque pas ; `TerminalEvent::Closed` suit.
    pub fn close(&self) {
        self.shared.closing.store(true, Ordering::Release);
        if let Some(pty) = self.shared.pty.get() {
            pty.kill_tree();
        }
        let _ = self.commands.send(Command::Close);
    }

    /// `close` puis attente de la fin (au plus `timeout`). **Bloquant** : tests
    /// et fermeture de l'IDE seulement. Rend `true` si tout est arrêté.
    pub fn close_and_wait(&self, timeout: Duration) -> bool {
        self.close();
        self.wait(timeout)
    }

    /// Attend la fin de la session (au plus `timeout`). **Bloquant**.
    pub fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.shared.finished.load(Ordering::Acquire) && self.task.is_finished() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.shared.finished.load(Ordering::Acquire) {
            self.close();
        }
    }
}
