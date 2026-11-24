//! Pseudo-console Windows (ConPTY) et objet « job » qui tient tout l'arbre de
//! processus de l'interpréteur.
//!
//! Ordre de fermeture (voir [`Pty::shutdown`]) : on tue d'abord l'arbre (job),
//! puis on ferme la pseudo-console pendant que le fil de lecture continue de
//! vider la sortie ; sinon `ClosePseudoConsole` peut attendre indéfiniment que
//! conhost ait fini d'écrire.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use crate::error::TerminalError;

/// Ce qu'il faut pour lancer un interpréteur.
#[derive(Debug, Clone)]
pub(crate) struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// Variables ajoutées ou remplacées par-dessus l'environnement hérité.
    pub env: Vec<(OsString, OsString)>,
    pub columns: u16,
    pub lines: u16,
}

/// Poignée Win32 fermée à la destruction.
struct OwnedHandle(HANDLE);

// SAFETY : une poignée Win32 est un identifiant de noyau, utilisable depuis
// n'importe quel fil ; les accès concurrents sont sérialisés par nos `Mutex`.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    fn new(handle: HANDLE) -> Option<Self> {
        (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(Self(handle))
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY : poignée valide, possédée par nous seuls.
        unsafe { CloseHandle(self.0) };
    }
}

fn last_error(call: &'static str) -> TerminalError {
    // SAFETY : sans précondition.
    let code = unsafe { GetLastError() };
    TerminalError::Os { call, code }
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Une pseudo-console et l'arbre de processus qui y est attaché.
pub(crate) struct Pty {
    console: Mutex<HPCON>,
    input: Mutex<Option<OwnedHandle>>,
    process: OwnedHandle,
    pid: u32,
    job: OwnedHandle,
}

/// Côté lecture de la sortie de la pseudo-console (un seul fil la lit).
pub(crate) struct PtyReader(OwnedHandle);

impl PtyReader {
    /// Lecture bloquante ; `Ok(0)` à la fin (pseudo-console fermée).
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, TerminalError> {
        let mut read = 0u32;
        let len = buffer.len().min(u32::MAX as usize) as u32;
        // SAFETY : tampon valide de `len` octets, poignée de lecture valide.
        let ok = unsafe {
            ReadFile(
                self.0 .0,
                buffer.as_mut_ptr(),
                len,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            // SAFETY : sans précondition.
            let code = unsafe { GetLastError() };
            if code == ERROR_BROKEN_PIPE {
                return Ok(0);
            }
            return Err(TerminalError::Os {
                call: "ReadFile",
                code,
            });
        }
        Ok(read as usize)
    }
}

/// Ligne de commande selon les règles de découpage de la bibliothèque C de
/// Microsoft (celles qu'appliquent cmd, bash.exe et PowerShell).
pub(crate) fn command_line(program: &Path, args: &[OsString]) -> Vec<u16> {
    let mut line: Vec<u16> = Vec::new();
    // Le programme : entre guillemets, sans échappement (un nom de fichier ne
    // contient pas de guillemet).
    line.push('"' as u16);
    line.extend(program.as_os_str().encode_wide());
    line.push('"' as u16);
    for arg in args {
        line.push(' ' as u16);
        quote_arg(&mut line, arg);
    }
    line.push(0);
    line
}

fn quote_arg(line: &mut Vec<u16>, arg: &OsStr) {
    let units: Vec<u16> = arg.encode_wide().collect();
    let needs_quotes = units.is_empty()
        || units
            .iter()
            .any(|&u| u == ' ' as u16 || u == '\t' as u16 || u == '"' as u16);
    if !needs_quotes {
        line.extend_from_slice(&units);
        return;
    }
    line.push('"' as u16);
    let mut backslashes = 0usize;
    for &u in &units {
        if u == '\\' as u16 {
            backslashes += 1;
            continue;
        }
        if u == '"' as u16 {
            line.extend(std::iter::repeat_n('\\' as u16, backslashes * 2 + 1));
        } else {
            line.extend(std::iter::repeat_n('\\' as u16, backslashes));
        }
        backslashes = 0;
        line.push(u);
    }
    line.extend(std::iter::repeat_n('\\' as u16, backslashes * 2));
    line.push('"' as u16);
}

/// Bloc d'environnement UTF-16 : l'environnement hérité, plus `extra`
/// (remplace une variable de même nom, sans casse), trié comme Windows l'attend.
fn environment_block(extra: &[(OsString, OsString)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(key, _)| {
            !extra.iter().any(|(k, _)| {
                k.to_string_lossy()
                    .eq_ignore_ascii_case(&key.to_string_lossy())
            })
        })
        .collect();
    vars.extend(extra.iter().cloned());
    vars.sort_by_key(|(key, _)| key.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (key, value) in vars {
        block.extend(key.encode_wide());
        block.push('=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

fn coord(columns: u16, lines: u16) -> COORD {
    COORD {
        X: columns.clamp(1, i16::MAX as u16) as i16,
        Y: lines.clamp(1, i16::MAX as u16) as i16,
    }
}

impl Pty {
    /// Crée la pseudo-console et lance l'interpréteur dedans. Bloquant
    /// (lancement de processus) : à appeler dans une tâche.
    pub fn spawn(spec: &SpawnSpec) -> Result<(Pty, PtyReader), TerminalError> {
        if !spec.program.is_file() {
            return Err(TerminalError::ShellNotFound(spec.program.clone()));
        }
        if !spec.cwd.is_dir() {
            return Err(TerminalError::DirectoryNotFound(spec.cwd.clone()));
        }

        // Deux tuyaux : entrée (nous → conhost) et sortie (conhost → nous).
        let mut input_read: HANDLE = std::ptr::null_mut();
        let mut input_write: HANDLE = std::ptr::null_mut();
        let mut output_read: HANDLE = std::ptr::null_mut();
        let mut output_write: HANDLE = std::ptr::null_mut();
        // SAFETY : pointeurs de sortie valides.
        unsafe {
            if CreatePipe(&mut input_read, &mut input_write, std::ptr::null(), 0) == 0 {
                return Err(last_error("CreatePipe"));
            }
        }
        let input_read = OwnedHandle(input_read);
        let input_write = OwnedHandle(input_write);
        // SAFETY : idem.
        unsafe {
            if CreatePipe(&mut output_read, &mut output_write, std::ptr::null(), 0) == 0 {
                return Err(last_error("CreatePipe"));
            }
        }
        let output_read = OwnedHandle(output_read);
        let output_write = OwnedHandle(output_write);

        let mut console: HPCON = 0;
        // SAFETY : poignées de tuyau valides ; conhost les duplique.
        let hr = unsafe {
            CreatePseudoConsole(
                coord(spec.columns, spec.lines),
                input_read.0,
                output_write.0,
                0,
                &mut console,
            )
        };
        if hr < 0 {
            return Err(TerminalError::Os {
                call: "CreatePseudoConsole",
                code: hr as u32,
            });
        }
        // conhost a ses propres copies : les nôtres ne serviraient qu'à empêcher
        // la fin de fichier de nous parvenir.
        drop(input_read);
        drop(output_write);

        match Self::spawn_process(spec, console) {
            Ok((process, pid, job)) => Ok((
                Pty {
                    console: Mutex::new(console),
                    input: Mutex::new(Some(input_write)),
                    process,
                    pid,
                    job,
                },
                PtyReader(output_read),
            )),
            Err(error) => {
                // SAFETY : console créée ci-dessus, jamais fermée.
                unsafe { ClosePseudoConsole(console) };
                Err(error)
            }
        }
    }

    fn spawn_process(
        spec: &SpawnSpec,
        console: HPCON,
    ) -> Result<(OwnedHandle, u32, OwnedHandle), TerminalError> {
        // Liste d'attributs : un seul, la pseudo-console.
        let mut size = 0usize;
        // SAFETY : premier appel pour connaître la taille (échoue exprès).
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
        let mut storage = vec![0u64; size.div_ceil(8).max(1)];
        let list = storage.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        // SAFETY : `storage` fait au moins `size` octets, aligné sur 8.
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        struct AttributeList(LPPROC_THREAD_ATTRIBUTE_LIST);
        impl Drop for AttributeList {
            fn drop(&mut self) {
                // SAFETY : liste initialisée.
                unsafe { DeleteProcThreadAttributeList(self.0) };
            }
        }
        let _list_guard = AttributeList(list);
        // SAFETY : la valeur de l'attribut est la poignée elle-même (convention
        // documentée de PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE).
        let ok = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                console as *const std::ffi::c_void,
                std::mem::size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(last_error("UpdateProcThreadAttribute"));
        }

        // SAFETY : structure C sans invariant ; zéro est un état valide.
        let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = list;
        // Entrées-sorties standard nulles : sans cela, si l'IDE a lui-même une
        // console (lancé depuis un terminal), l'enfant hériterait de celle-ci au
        // lieu de la pseudo-console.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;

        let mut command = command_line(&spec.program, &spec.args);
        let environment = environment_block(&spec.env);
        let cwd = wide(spec.cwd.as_os_str());
        // SAFETY : structure de sortie.
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY : chaînes terminées par zéro, tampons vivants pendant l'appel ;
        // aucune poignée héritée.
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
                environment.as_ptr() as *const std::ffi::c_void,
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut info,
            )
        };
        if ok == 0 {
            // SAFETY : sans précondition.
            let code = unsafe { GetLastError() };
            return Err(
                if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
                    TerminalError::ShellNotFound(spec.program.clone())
                } else {
                    TerminalError::Os {
                        call: "CreateProcessW",
                        code,
                    }
                },
            );
        }
        let process = OwnedHandle(info.hProcess);
        let thread = OwnedHandle(info.hThread);

        // Le job avant de laisser partir le processus : aucun descendant ne
        // peut lui échapper. KILL_ON_JOB_CLOSE : si l'IDE meurt, Windows ferme
        // la poignée et tue tout l'arbre.
        let job = match create_job() {
            Ok(job) => job,
            Err(error) => {
                // SAFETY : processus suspendu que nous possédons.
                unsafe { TerminateProcess(process.0, 1) };
                return Err(error);
            }
        };
        // SAFETY : poignées valides.
        if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
            let error = last_error("AssignProcessToJobObject");
            // SAFETY : idem.
            unsafe { TerminateProcess(process.0, 1) };
            return Err(error);
        }
        // SAFETY : fil principal suspendu par CREATE_SUSPENDED.
        unsafe { ResumeThread(thread.0) };
        drop(thread);
        Ok((process, info.dwProcessId, job))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Écrit tout `bytes` sur l'entrée de la pseudo-console. Bloquant si conhost
    /// ne lit plus : à appeler depuis le fil de contrôle de la session.
    pub fn write_all(&self, mut bytes: &[u8]) -> Result<(), TerminalError> {
        let guard = self.input.lock().unwrap_or_else(|e| e.into_inner());
        let Some(input) = guard.as_ref() else {
            return Ok(());
        };
        while !bytes.is_empty() {
            let mut written = 0u32;
            let len = bytes.len().min(64 * 1024) as u32;
            // SAFETY : tampon valide de `len` octets.
            let ok = unsafe {
                WriteFile(
                    input.0,
                    bytes.as_ptr(),
                    len,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(last_error("WriteFile"));
            }
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }

    /// Nouvelle taille de la pseudo-console (en cellules).
    pub fn resize(&self, columns: u16, lines: u16) -> Result<(), TerminalError> {
        let console = self.console.lock().unwrap_or_else(|e| e.into_inner());
        if *console == 0 {
            return Ok(());
        }
        // SAFETY : console ouverte (protégée par le verrou).
        let hr = unsafe { ResizePseudoConsole(*console, coord(columns, lines)) };
        if hr < 0 {
            return Err(TerminalError::Os {
                call: "ResizePseudoConsole",
                code: hr as u32,
            });
        }
        Ok(())
    }

    /// Code de sortie si l'interpréteur s'est terminé.
    pub fn try_wait(&self) -> Option<u32> {
        // SAFETY : poignée de processus valide.
        unsafe {
            if WaitForSingleObject(self.process.0, 0) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            GetExitCodeProcess(self.process.0, &mut code);
            Some(code)
        }
    }

    /// Attend la fin de l'interpréteur au plus `timeout_ms`.
    pub fn wait(&self, timeout_ms: u32) -> Option<u32> {
        // SAFETY : poignée de processus valide.
        unsafe { WaitForSingleObject(self.process.0, timeout_ms) };
        self.try_wait()
    }

    /// Tue l'interpréteur et tous ses descendants. Ne bloque pas.
    pub fn kill_tree(&self) {
        // SAFETY : poignée de job valide.
        unsafe { TerminateJobObject(self.job.0, 1) };
    }

    /// Tue l'arbre puis ferme la pseudo-console (conhost s'arrête, la lecture
    /// rend la fin de fichier). Le fil de lecture doit continuer de vider la
    /// sortie pendant cet appel. Idempotent.
    pub fn shutdown(&self) {
        self.kill_tree();
        {
            let mut console = self.console.lock().unwrap_or_else(|e| e.into_inner());
            if *console != 0 {
                // SAFETY : console ouverte, fermée une seule fois (verrou).
                unsafe { ClosePseudoConsole(*console) };
                *console = 0;
            }
        }
        self.input.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    /// Identifiants des processus vivants de l'arbre (l'interpréteur compris).
    pub fn process_ids(&self) -> Vec<u32> {
        // JOBOBJECT_BASIC_PROCESS_ID_LIST : deux u32 puis un tableau de usize.
        const CAPACITY: usize = 1024;
        let mut buffer = vec![0usize; CAPACITY + 1];
        // SAFETY : tampon de (CAPACITY + 1) * size_of::<usize>() octets.
        let ok = unsafe {
            QueryInformationJobObject(
                self.job.0,
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr() as *mut std::ffi::c_void,
                (buffer.len() * std::mem::size_of::<usize>()) as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Vec::new();
        }
        // SAFETY : l'en-tête occupe le premier usize (deux u32).
        let listed = unsafe { *(buffer.as_ptr() as *const u32).add(1) } as usize;
        buffer[1..1 + listed.min(CAPACITY)]
            .iter()
            .map(|&pid| pid as u32)
            .collect()
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn create_job() -> Result<OwnedHandle, TerminalError> {
    // SAFETY : job anonyme.
    let job = OwnedHandle::new(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })
        .ok_or_else(|| last_error("CreateJobObjectW"))?;
    // SAFETY : structure C, zéro valide.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY : structure de la bonne taille pour cette classe d'information.
    let ok = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(last_error("SetInformationJobObject"));
    }
    Ok(job)
}

/// Un processus de la machine (instantané ToolHelp).
#[derive(Debug, Clone)]
pub(crate) struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

/// Instantané de tous les processus (quelques millisecondes).
pub(crate) fn process_snapshot() -> Vec<ProcessEntry> {
    let mut entries = Vec::new();
    // SAFETY : instantané fermé par OwnedHandle.
    let Some(snapshot) =
        OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
    else {
        return entries;
    };
    // SAFETY : structure C, zéro valide ; dwSize renseigné.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    // SAFETY : instantané et structure valides.
    let mut ok = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while ok != 0 {
        let len = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        entries.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent: entry.th32ParentProcessID,
            name: String::from_utf16_lossy(&entry.szExeFile[..len]),
        });
        // SAFETY : idem.
        ok = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let units = command_line(Path::new(program), &args);
        String::from_utf16(&units[..units.len() - 1]).unwrap()
    }

    #[test]
    fn command_line_quotes_like_msvcrt() {
        assert_eq!(
            line(r"C:\Program Files\Git\bin\bash.exe", &["--login", "-i"]),
            r#""C:\Program Files\Git\bin\bash.exe" --login -i"#
        );
        assert_eq!(
            line("cmd.exe", &["/c", "echo a b"]),
            r#""cmd.exe" /c "echo a b""#
        );
        assert_eq!(line("x", &[r#"a"b"#]), r#""x" "a\"b""#);
        assert_eq!(
            line("x", &[r"C:\dir with space\"]),
            r#""x" "C:\dir with space\\""#
        );
        assert_eq!(line("x", &[""]), r#""x" """#);
    }

    #[test]
    fn environment_block_replaces_case_insensitively() {
        let block = environment_block(&[(OsString::from("path"), OsString::from("X"))]);
        let text = String::from_utf16(&block).unwrap();
        let vars: Vec<&str> = text.split('\0').filter(|v| !v.is_empty()).collect();
        let paths: Vec<&&str> = vars
            .iter()
            .filter(|v| v.to_ascii_uppercase().starts_with("PATH="))
            .collect();
        assert_eq!(paths, vec![&"path=X"]);
        assert!(text.ends_with("\0\0"));
    }
}
