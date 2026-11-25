//! Sessions réelles dans une pseudo-console, sans fenêtre : cmd (toujours),
//! Git Bash (s'il est installé). Les essais tournent l'un après l'autre (verrou)
//! pour pouvoir compter les conhost lancés par ce processus.

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use julie_core::bus::Bus;
use julie_core::{Domain, Event, Tasks};
use julie_terminal::{
    profile, Key, Modifiers, Profile, ProfileKind, Screen, Session, SessionOptions, SessionStatus,
    TerminalEvent, TerminalSettings, Terminals,
};
use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

static SERIAL: Mutex<()> = Mutex::new(());

const TIMEOUT: Duration = Duration::from_secs(20);

struct Fixture {
    bus: Bus,
    terminals: Terminals,
    dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let bus = Bus::new();
    let tasks = Tasks::new(bus.publisher());
    // Un chemin avec espaces et parenthèses, comme « Program Files (x86) ».
    let dir = tempfile::Builder::new()
        .prefix("julie terminal (essai) ")
        .tempdir()
        .unwrap();
    Fixture {
        bus,
        terminals: Terminals::new(tasks, TerminalSettings::default()),
        dir,
    }
}

impl Fixture {
    fn open(&self, profile: Profile) -> Session {
        self.terminals
            .open(SessionOptions::new(profile, self.dir.path()))
    }

    /// Attend un événement du terminal qui vérifie `pred`.
    fn wait_event(&self, mut pred: impl FnMut(&TerminalEvent) -> bool) -> bool {
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if let Some(Event::Domain(event)) = self.bus.recv_timeout(Duration::from_millis(50)) {
                if event.domain == Domain::Terminal {
                    if let Some(e) = event.get::<TerminalEvent>() {
                        if pred(e) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

fn cmd() -> Profile {
    profile(ProfileKind::Cmd, &TerminalSettings::default())
}

fn cmd_with(args: &[&str]) -> Profile {
    let mut profile = cmd();
    profile.args = args.iter().map(|a| a.to_string()).collect();
    profile
}

fn git_bash() -> Option<Profile> {
    let profile = profile(ProfileKind::GitBash, &TerminalSettings::default());
    profile.program.is_file().then_some(profile)
}

/// Attend que l'écran vérifie `pred` ; rend le dernier écran.
fn wait_screen(session: &Session, pred: impl Fn(&str) -> bool) -> (bool, String) {
    let deadline = Instant::now() + TIMEOUT;
    let mut screen = Screen::default();
    loop {
        session.snapshot(&mut screen);
        let text = screen.text();
        if pred(&text) {
            return (true, text);
        }
        if Instant::now() >= deadline {
            return (false, text);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_status(session: &Session, pred: impl Fn(&SessionStatus) -> bool) -> SessionStatus {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let status = session.status();
        if pred(&status) || Instant::now() >= deadline {
            return status;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn type_line(session: &Session, line: &str) {
    session.send_text(line);
    session.send_key(Key::Enter, Modifiers::NONE);
}

fn is_alive(pid: u32) -> bool {
    // SAFETY : appels Win32 sur une poignée que nous fermons.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        ok != 0 && code == STILL_ACTIVE as u32
    }
}

/// (pid, parent, nom) de tous les processus.
fn processes() -> Vec<(u32, u32, String)> {
    let mut list = Vec::new();
    // SAFETY : instantané ToolHelp fermé à la fin.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            list.push((
                entry.th32ProcessID,
                entry.th32ParentProcessID,
                String::from_utf16_lossy(&entry.szExeFile[..len]),
            ));
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    list
}

/// conhost / OpenConsole lancés par ce processus (un par pseudo-console ouverte).
fn my_conhosts() -> Vec<u32> {
    let me = std::process::id();
    processes()
        .into_iter()
        .filter(|(_, parent, name)| {
            *parent == me
                && (name.eq_ignore_ascii_case("conhost.exe")
                    || name.eq_ignore_ascii_case("OpenConsole.exe"))
        })
        .map(|(pid, _, _)| pid)
        .collect()
}

fn wait_until(mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    pred()
}

fn path_text(path: &Path) -> String {
    path.display().to_string()
}

#[test]
fn cmd_echo_output_and_exit_code() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let session = f.open(cmd_with(&["/c", "echo bonjour é"]));
    let id = session.id();
    assert!(f.wait_event(|e| matches!(e, TerminalEvent::Started { session, .. } if *session == id)));
    assert!(
        f.wait_event(|e| matches!(e, TerminalEvent::Exited { session, code: 0 } if *session == id))
    );
    let (found, text) = wait_screen(&session, |t| t.contains("bonjour é"));
    assert!(found, "écran : {text:?}");
    assert!(matches!(
        session.status(),
        SessionStatus::Exited { code: 0 }
    ));
    assert!(session.wait(TIMEOUT));
    assert!(session.process_ids().is_empty());
}

#[test]
fn cmd_exit_code_is_reported() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let session = f.open(cmd_with(&["/c", "exit 3"]));
    let id = session.id();
    assert!(
        f.wait_event(|e| matches!(e, TerminalEvent::Exited { session, code: 3 } if *session == id))
    );
}

#[test]
fn cmd_starts_in_given_directory_with_spaces_and_parentheses() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let session = f.open(cmd());
    let dir = path_text(f.dir.path());
    let (found, text) = wait_screen(&session, |t| t.contains(&format!("{dir}>")));
    assert!(found, "invite attendue dans {dir:?}, écran : {text:?}");
    type_line(&session, "cd");
    let (found, text) = wait_screen(&session, |t| t.lines().any(|l| l.trim() == dir));
    assert!(found, "écran : {text:?}");
    type_line(&session, "exit");
    assert!(wait_status(&session, |s| s.is_finished()).is_finished());
    assert!(matches!(
        session.status(),
        SessionStatus::Exited { code: 0 }
    ));
}

#[test]
fn cmd_resize_reaches_the_console() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let session = f.open(cmd());
    let (found, _) = wait_screen(&session, |t| t.contains('>'));
    assert!(found);
    session.resize(101, 33, 8, 16);
    let mut screen = Screen::default();
    assert!(wait_until(|| {
        session.snapshot(&mut screen);
        screen.columns == 101 && screen.lines == 33
    }));
    type_line(&session, "%SystemRoot%\\System32\\mode.com con");
    // « Colonnes : 101 » / « Columns: 101 » selon la langue de Windows.
    let (found, text) = wait_screen(&session, |t| {
        t.lines().any(|l| l.trim_end().ends_with(" 101"))
            && t.lines().any(|l| l.trim_end().ends_with(" 33"))
    });
    assert!(found, "écran : {text:?}");
    assert!(session.close_and_wait(TIMEOUT));
}

#[test]
fn ctrl_c_interrupts_a_long_command() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let session = f.open(cmd());
    assert!(wait_screen(&session, |t| t.contains('>')).0);
    type_line(&session, "%SystemRoot%\\System32\\PING.EXE -t 127.0.0.1");
    assert!(
        wait_until(|| session.running_programs().contains(&"ping".to_string())),
        "ping devrait tourner"
    );
    let (found, _) = wait_screen(&session, |t| t.matches("127.0.0.1").count() >= 3);
    assert!(found);
    assert!(session.send_key(Key::Char('c'), Modifiers::CTRL));
    assert!(
        wait_until(|| session.running_programs().is_empty()),
        "ping devrait être interrompu : {:?}",
        session.running_programs()
    );
    // Le shell, lui, tourne toujours.
    assert!(matches!(session.status(), SessionStatus::Running { .. }));
    assert!(session.close_and_wait(TIMEOUT));
}

#[test]
fn closing_kills_the_whole_tree_and_conhost() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = my_conhosts();
    let f = fixture();
    let session = f.open(cmd());
    assert!(wait_screen(&session, |t| t.contains('>')).0);
    // Un petit-enfant : cmd → cmd → ping.
    type_line(
        &session,
        "cmd /c %SystemRoot%\\System32\\PING.EXE -t 127.0.0.1",
    );
    assert!(wait_until(|| session.process_ids().len() >= 3));
    let tree = session.process_ids();
    let shell = session.pid().unwrap();
    assert!(tree.contains(&shell));
    assert!(my_conhosts().len() > before.len(), "conhost attendu");
    let names = session.running_programs();
    assert!(names.contains(&"ping".to_string()), "{names:?}");

    let id = session.id();
    session.close();
    assert!(f.wait_event(|e| matches!(e, TerminalEvent::Closed { session } if *session == id)));
    assert!(session.wait(TIMEOUT));
    assert!(matches!(session.status(), SessionStatus::Closed));
    for pid in &tree {
        assert!(
            wait_until(|| !is_alive(*pid)),
            "processus {pid} encore vivant"
        );
    }
    assert!(
        wait_until(|| my_conhosts().len() <= before.len()),
        "conhost orphelin : {:?}",
        my_conhosts()
    );
}

#[test]
fn dropping_a_session_closes_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = my_conhosts();
    let f = fixture();
    let session = f.open(cmd());
    assert!(wait_screen(&session, |t| t.contains('>')).0);
    let shell = session.pid().unwrap();
    let id = session.id();
    drop(session);
    assert!(f.wait_event(|e| matches!(e, TerminalEvent::Closed { session } if *session == id)));
    assert!(wait_until(|| !is_alive(shell)));
    assert!(wait_until(|| my_conhosts().len() <= before.len()));
}

#[test]
fn missing_shell_fails_cleanly() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let mut profile = cmd();
    profile.program = PathBuf::from(r"Z:\nulle\part\bash.exe");
    let session = f.open(profile);
    let id = session.id();
    assert!(f.wait_event(|e| matches!(e, TerminalEvent::Failed { session, .. } if *session == id)));
    assert!(matches!(session.status(), SessionStatus::Failed(_)));
    // Saisie et fermeture sans effet ni panique.
    session.send_text("x");
    session.resize(10, 10, 8, 16);
    assert!(session.close_and_wait(TIMEOUT));
}

#[test]
fn input_sent_before_start_is_kept() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let session = f.open(cmd());
    type_line(&session, "echo tôt-venu");
    let (found, text) = wait_screen(&session, |t| t.lines().any(|l| l.trim() == "tôt-venu"));
    assert!(found, "écran : {text:?}");
    assert!(session.close_and_wait(TIMEOUT));
}

#[test]
fn several_sessions_are_independent() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let a = f.open(cmd());
    let b = f.open(cmd());
    assert_ne!(a.id(), b.id());
    assert!(wait_screen(&a, |t| t.contains('>')).0);
    assert!(wait_screen(&b, |t| t.contains('>')).0);
    type_line(&a, "%SystemRoot%\\System32\\PING.EXE -t 127.0.0.1");
    type_line(&b, "echo dans-b");
    let (found, _) = wait_screen(&b, |t| t.lines().any(|l| l.trim() == "dans-b"));
    assert!(found);
    let (_, text_a) = wait_screen(&a, |t| t.contains("127.0.0.1"));
    assert!(!text_a.contains("dans-b"));
    assert!(a.close_and_wait(TIMEOUT));
    assert!(b.close_and_wait(TIMEOUT));
}

#[test]
fn git_bash_session() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(profile) = git_bash() else {
        eprintln!("Git Bash absent : essai sauté");
        return;
    };
    let before = my_conhosts();
    let f = fixture();
    let session = f.open(profile);
    session.resize(90, 20, 8, 16);
    // Dossier de départ : la racine donnée, pas $HOME.
    type_line(
        &session,
        "pwd; echo \"cols=$(tput cols) lines=$(tput lines) term=$TERM\"; echo é",
    );
    let (found, text) = wait_screen(&session, |t| {
        t.contains("cols=90 lines=20 term=xterm-256color") && t.lines().any(|l| l.trim() == "é")
    });
    assert!(found, "écran : {text:?}");
    let name = f
        .dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        text.lines()
            .any(|l| l.trim().starts_with("/") && l.trim().ends_with(&name)),
        "pwd attendu dans {name:?} : {text:?}"
    );

    // Couleurs : `ls --color` et 256 couleurs.
    std::fs::write(f.dir.path().join("fichier.txt"), "x").unwrap();
    std::fs::create_dir(f.dir.path().join("dossier")).unwrap();
    type_line(
        &session,
        "clear; ls --color=always; printf '\\e[38;5;208mORANGE\\e[0m\\n'",
    );
    let mut screen = Screen::default();
    assert!(wait_until(|| {
        session.snapshot(&mut screen);
        let text = screen.text();
        text.contains("ORANGE") && text.contains("dossier")
    }));
    let colored = |word: &str| {
        (0..screen.lines).any(|l| {
            let line = screen.line_text(l);
            line.find(word).is_some_and(|byte| {
                let column = line[..byte].chars().count();
                screen.row(l)[column].fg != julie_terminal::Color::Foreground
            })
        })
    };
    assert!(colored("dossier"), "dossier en couleur : {}", screen.text());
    assert!(colored("ORANGE"));
    assert!((0..screen.lines).any(|l| screen
        .row(l)
        .iter()
        .any(|c| c.fg == julie_terminal::Color::Indexed(208))));

    // Ctrl+C interrompt sleep.
    type_line(&session, "sleep 100");
    assert!(wait_until(|| session
        .running_programs()
        .contains(&"sleep".to_string())));
    session.send_key(Key::Char('c'), Modifiers::CTRL);
    assert!(
        wait_until(|| session.running_programs().is_empty()),
        "{:?}",
        session.running_programs()
    );

    // Fermer pendant un sleep : tout l'arbre disparaît.
    type_line(&session, "sleep 100");
    assert!(wait_until(|| session
        .running_programs()
        .contains(&"sleep".to_string())));
    let tree = session.process_ids();
    assert!(session.close_and_wait(TIMEOUT));
    for pid in &tree {
        assert!(
            wait_until(|| !is_alive(*pid)),
            "processus {pid} encore vivant"
        );
    }
    assert!(wait_until(|| my_conhosts().len() <= before.len()));
}

#[test]
fn git_bash_exit_codes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(profile) = git_bash() else {
        return;
    };
    let f = fixture();
    let session = f.open(profile.clone());
    type_line(&session, "exit 3");
    let status = wait_status(&session, |s| s.is_finished());
    assert!(
        matches!(status, SessionStatus::Exited { code: 3 }),
        "{status:?}"
    );

    let session = f.open(profile);
    type_line(&session, "exit");
    let status = wait_status(&session, |s| s.is_finished());
    assert!(
        matches!(status, SessionStatus::Exited { code: 0 }),
        "{status:?}"
    );
}

#[test]
fn git_bash_alternate_screen_with_less() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(profile) = git_bash() else {
        return;
    };
    let f = fixture();
    let session = f.open(profile);
    session.resize(80, 15, 8, 16);
    type_line(&session, "echo avant-less; seq 1 200 | less");
    let mut screen = Screen::default();
    assert!(wait_until(|| {
        session.snapshot(&mut screen);
        screen.modes.alt_screen && screen.text().lines().any(|l| l.trim() == "1")
    }));
    // La molette envoie des flèches à less.
    session.wheel(-3);
    assert!(
        wait_until(|| {
            session.snapshot(&mut screen);
            screen.line_text(0).trim() == "4"
        }),
        "{}",
        screen.text()
    );
    session.send_text("q");
    assert!(wait_until(|| {
        session.snapshot(&mut screen);
        !screen.modes.alt_screen && screen.text().contains("avant-less")
    }));
    assert!(session.close_and_wait(TIMEOUT));
}

/// Le processus courant ignore-t-il Ctrl+C (bit 0 de
/// `RTL_USER_PROCESS_PARAMETERS.ConsoleFlags`, hérité des parents) ?
fn ctrl_c_ignored() -> bool {
    use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, PROCESS_BASIC_INFORMATION};
    // SAFETY : lecture de notre propre PEB (voir src/console_ctrl.rs).
    unsafe {
        let mut info: PROCESS_BASIC_INFORMATION = std::mem::zeroed();
        let status = NtQueryInformationProcess(
            GetCurrentProcess(),
            ProcessBasicInformation,
            &mut info as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            std::ptr::null_mut(),
        );
        assert!(status >= 0);
        let params = (*info.PebBaseAddress).ProcessParameters;
        (std::ptr::read_volatile(&(*params).Reserved2[1]) as usize as u32) & 1 != 0
    }
}

/// Lance ce même exécutable de test sur `helper`, avec les drapeaux de
/// création `flags` ; rend (succès, sortie standard, pid de l'hôte).
fn run_helper(helper: &str, variable: &str, flags: u32) -> (bool, String, u32) {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            helper,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(variable, "1")
        .creation_flags(flags)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let host = child.id();
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    let status = child.wait().unwrap();
    (status.success(), output, host)
}

/// Hôte de l'essai suivant : né en ignorant Ctrl+C (comme un IDE lancé depuis
/// le Git Bash de l'outil d'agent), il vérifie que Ctrl+C interrompt quand même
/// les commandes de ses terminaux, et que lui-même ignore toujours Ctrl+C.
#[test]
#[ignore = "lancé par ctrl_c_works_when_the_ide_ignores_ctrl_c"]
fn helper_ctrl_c_under_an_ignoring_parent() {
    if std::env::var_os("JULIE_TERMINAL_HELPER_CTRL_C").is_none() {
        return;
    }
    use std::io::Write;
    println!("IGNORED_AT_START {}", ctrl_c_ignored());
    let f = fixture();

    let session = f.open(cmd());
    assert!(wait_screen(&session, |t| t.contains('>')).0);
    type_line(&session, "%SystemRoot%\\System32\\PING.EXE -t 127.0.0.1");
    assert!(wait_until(|| session
        .running_programs()
        .contains(&"ping".to_string())));
    assert!(wait_screen(&session, |t| t.matches("127.0.0.1").count() >= 3).0);
    session.send_key(Key::Char('c'), Modifiers::CTRL);
    assert!(
        wait_until(|| session.running_programs().is_empty()),
        "ping devrait être interrompu : {:?}",
        session.running_programs()
    );
    assert!(matches!(session.status(), SessionStatus::Running { .. }));
    assert!(session.close_and_wait(TIMEOUT));
    println!("CMD_OK");

    if let Some(profile) = git_bash() {
        let session = f.open(profile);
        type_line(&session, "sleep 100");
        assert!(wait_until(|| session
            .running_programs()
            .contains(&"sleep".to_string())));
        session.send_key(Key::Char('c'), Modifiers::CTRL);
        assert!(
            wait_until(|| session.running_programs().is_empty()),
            "sleep devrait être interrompu : {:?}",
            session.running_programs()
        );
        assert!(session.close_and_wait(TIMEOUT));
        println!("BASH_OK");
    }

    // L'hôte lui-même ignore toujours Ctrl+C. Il a sa propre console cachée
    // (CREATE_NO_WINDOW) : on ne l'envoie que si personne d'autre n'y est
    // attaché, pour ne jamais toucher le shell qui a lancé les essais.
    use windows_sys::Win32::System::Console::{
        GenerateConsoleCtrlEvent, GetConsoleProcessList, CTRL_C_EVENT,
    };
    let mut pids = [0u32; 8];
    // SAFETY : tampon de 8 identifiants.
    let attached = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) };
    if attached == 1 && pids[0] == std::process::id() {
        // SAFETY : signal à notre seule console.
        let sent = unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) } != 0;
        std::thread::sleep(Duration::from_millis(500));
        println!("SELF_CTRL_C sent={sent} survived");
    } else {
        println!("SELF_CTRL_C skipped attached={attached}");
    }
    std::io::stdout().flush().unwrap();
}

/// Écart M2 : un IDE lancé par un parent qui ignore Ctrl+C (Git Bash de
/// l'outil d'agent, raccourci « nouveau groupe de processus »…) en hérite, et
/// le transmettait à cmd et à ping dans la pseudo-console. On reproduit ce
/// parent avec CREATE_NEW_PROCESS_GROUP, quel que soit le shell qui lance
/// `cargo test`.
#[test]
fn ctrl_c_works_when_the_ide_ignores_ctrl_c() {
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (ok, output, _) = run_helper(
        "helper_ctrl_c_under_an_ignoring_parent",
        "JULIE_TERMINAL_HELPER_CTRL_C",
        CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW,
    );
    assert!(
        output.contains("IGNORED_AT_START true"),
        "l'hôte devait naître en ignorant Ctrl+C : {output}"
    );
    assert!(ok && output.contains("CMD_OK"), "{output}");
    if git_bash().is_some() {
        assert!(output.contains("BASH_OK"), "{output}");
    }
    assert!(
        output.contains("SELF_CTRL_C sent=true survived"),
        "l'IDE doit continuer d'ignorer Ctrl+C : {output}"
    );
}

/// Hôte de l'essai suivant : ouvre des sessions occupées (et une qui démarre à
/// peine), les ferme sans attendre et se termine aussitôt — comme l'IDE qu'on
/// quitte.
#[test]
#[ignore = "lancé par closing_without_waiting_then_exiting_leaves_no_orphan"]
fn helper_close_without_waiting_and_exit() {
    if std::env::var_os("JULIE_TERMINAL_HELPER_EXIT").is_none() {
        return;
    }
    let f = fixture();
    let busy = f.open(cmd());
    assert!(wait_screen(&busy, |t| t.contains('>')).0);
    type_line(
        &busy,
        "cmd /c %SystemRoot%\\System32\\PING.EXE -t 127.0.0.1",
    );
    assert!(wait_until(|| busy.process_ids().len() >= 3));
    let mut pids = busy.process_ids();
    let bash = git_bash().map(|profile| {
        let session = f.open(profile);
        type_line(&session, "sleep 100");
        assert!(wait_until(|| session
            .running_programs()
            .contains(&"sleep".to_string())));
        pids.extend(session.process_ids());
        session
    });
    // Ouverte puis fermée avant d'avoir démarré.
    let starting = f.open(cmd());
    let pids: Vec<String> = pids.iter().map(|p| p.to_string()).collect();
    println!("PIDS {}", pids.join(" "));
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    let before = Instant::now();
    busy.close();
    starting.close();
    if let Some(bash) = &bash {
        bash.close();
    }
    println!("CLOSE_TOOK_MS {}", before.elapsed().as_millis());
    std::io::stdout().flush().unwrap();
    std::process::exit(0);
}

/// Fermer sans attendre puis quitter : ni interpréteur, ni descendant, ni
/// conhost ne survit (tout ce dont l'hôte était le parent est mort).
#[test]
fn closing_without_waiting_then_exiting_leaves_no_orphan() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (ok, output, host) = run_helper(
        "helper_close_without_waiting_and_exit",
        "JULIE_TERMINAL_HELPER_EXIT",
        0,
    );
    assert!(ok, "{output}");
    let pids: Vec<u32> = output
        .split_once("PIDS ")
        .map(|(_, rest)| {
            rest.lines()
                .next()
                .unwrap_or("")
                .split(' ')
                .filter_map(|p| p.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    assert!(pids.len() >= 3, "identifiants attendus : {output}");
    let took: u128 = output
        .split_once("CLOSE_TOOK_MS ")
        .and_then(|(_, rest)| rest.lines().next()?.trim().parse().ok())
        .expect("durée de fermeture");
    assert!(took < 100, "close() a bloqué {took} ms");
    for pid in &pids {
        assert!(
            wait_until(|| !is_alive(*pid)),
            "processus {pid} a survécu à l'hôte"
        );
    }
    assert!(
        wait_until(|| !processes()
            .iter()
            .any(|(pid, parent, _)| *parent == host && is_alive(*pid))),
        "enfants de l'hôte encore vivants : {:?}",
        processes()
            .into_iter()
            .filter(|(_, parent, _)| *parent == host)
            .collect::<Vec<_>>()
    );
}

/// Processus « hôte » de l'essai suivant : ouvre une session qui lance un
/// petit-enfant, écrit les identifiants, puis attend d'être tué.
#[test]
#[ignore = "lancé par host_killed_leaves_no_orphan"]
fn helper_host_to_be_killed() {
    if std::env::var_os("JULIE_TERMINAL_HELPER").is_none() {
        return;
    }
    let f = fixture();
    let session = f.open(cmd());
    assert!(wait_screen(&session, |t| t.contains('>')).0);
    type_line(
        &session,
        "cmd /c %SystemRoot%\\System32\\PING.EXE -t 127.0.0.1",
    );
    assert!(wait_until(|| session.process_ids().len() >= 3));
    let pids: Vec<String> = session
        .process_ids()
        .iter()
        .map(|p| p.to_string())
        .collect();
    println!("PIDS {}", pids.join(" "));
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(120));
}

/// L'IDE meurt brutalement (plantage, fin de tâche) : ni l'interpréteur, ni ses
/// descendants, ni conhost ne lui survivent.
#[test]
fn host_killed_leaves_no_orphan() {
    use std::io::BufRead;
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "helper_host_to_be_killed",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("JULIE_TERMINAL_HELPER", "1")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let host = child.id();
    let stdout = child.stdout.take().unwrap();
    let mut pids: Vec<u32> = Vec::new();
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line.unwrap();
        // libtest écrit « test … ... » sur la même ligne, avant notre sortie.
        if let Some((_, rest)) = line.split_once("PIDS ") {
            pids = rest.split(' ').filter_map(|p| p.parse().ok()).collect();
            break;
        }
    }
    assert!(pids.len() >= 3, "identifiants attendus");
    let conhosts: Vec<u32> = processes()
        .into_iter()
        .filter(|(_, parent, name)| *parent == host && name.eq_ignore_ascii_case("conhost.exe"))
        .map(|(pid, _, _)| pid)
        .collect();
    assert!(!conhosts.is_empty(), "conhost de la pseudo-console attendu");
    child.kill().unwrap();
    let _ = child.wait();
    for pid in pids.iter().chain(&conhosts) {
        assert!(
            wait_until(|| !is_alive(*pid)),
            "processus {pid} a survécu à l'hôte"
        );
    }
}
