//! Mise à la corbeille Windows.
//!
//! Toute suppression de fichier demandée dans l'IDE passe par
//! [`move_to_trash`] : l'élément reste restaurable depuis la corbeille. Si la
//! corbeille ne peut pas le recevoir (lecteur réseau, corbeille désactivée),
//! **rien n'est supprimé** et l'appel rend [`Error::TrashUnavailable`] : c'est à
//! l'interface de demander une seconde confirmation avant une suppression
//! définitive, qu'elle fait alors elle-même.
//!
//! Bloquant (accès disque, appel au shell Windows) : tâche de fond.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Envoie des fichiers ou des dossiers (avec leur contenu) à la corbeille.
///
/// - Chemins **absolus**. Un chemin inexistant est une erreur ([`Error::Io`])
///   détectée avant toute suppression.
/// - Aucune fenêtre de Windows ne s'ouvre (ni confirmation, ni progression, ni
///   message d'erreur).
/// - L'opération s'arrête au premier échec. L'erreur dit ce qui est **encore sur
///   le disque** (`remaining`) : [`Error::TrashUnavailable`] si la corbeille ne
///   peut pas recevoir un élément, [`Error::TrashFailed`] pour tout autre échec
///   (fichier verrouillé, droits). Ce qui précède l'échec est déjà à la corbeille.
/// - Jamais de suppression définitive, dans aucun cas.
pub fn move_to_trash<P: AsRef<Path>>(paths: &[P]) -> Result<()> {
    let mut targets: Vec<PathBuf> = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(Error::Invalid(format!(
                "mise à la corbeille : chemin relatif refusé ({})",
                path.display()
            )));
        }
        fs::symlink_metadata(path).map_err(|source| Error::io(path, source))?;
        if !targets.iter().any(|known| known == path) {
            targets.push(path.to_path_buf());
        }
    }
    if targets.is_empty() {
        return Ok(());
    }

    let outcome = platform::recycle(&targets);
    let remaining: Vec<PathBuf> = targets
        .into_iter()
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .collect();
    match outcome {
        platform::Outcome::Unavailable => Err(Error::TrashUnavailable { remaining }),
        platform::Outcome::Failed(code) => Err(Error::TrashFailed { remaining, code }),
        // Le shell dit avoir réussi mais un élément est toujours là : on ne
        // prétend pas l'avoir supprimé.
        platform::Outcome::Done if !remaining.is_empty() => {
            Err(Error::TrashFailed { remaining, code: 0 })
        }
        platform::Outcome::Done => Ok(()),
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::PathBuf;

    pub(super) enum Outcome {
        #[allow(dead_code)]
        Done,
        Unavailable,
        #[allow(dead_code)]
        Failed(i32),
    }

    /// Seule la corbeille de Windows est prise en charge.
    pub(super) fn recycle(_paths: &[PathBuf]) -> Outcome {
        Outcome::Unavailable
    }
}

/// `IFileOperation` du shell Windows, appelé à la main (windows-sys ne fournit pas
/// les interfaces COM). Les tables de fonctions ci-dessous suivent l'ordre de
/// `ShObjIdl_core.h`.
///
/// Pourquoi pas `SHFileOperationW` : sans fenêtre de confirmation, elle supprime
/// **définitivement** ce que la corbeille ne peut pas recevoir. Ici, un
/// observateur ([`Sink`]) est consulté avant chaque suppression et la refuse si
/// Windows n'annonce pas une mise à la corbeille.
#[cfg(windows)]
mod platform {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;

    use windows_sys::core::{GUID, HRESULT, PCWSTR};
    use windows_sys::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
        COINIT_DISABLE_OLE1DDE,
    };
    use windows_sys::Win32::UI::Shell::SHCreateItemFromParsingName;

    pub(super) enum Outcome {
        Done,
        Unavailable,
        Failed(HRESULT),
    }

    const CLSID_FILE_OPERATION: GUID = GUID::from_u128(0x3ad05575_8857_4850_9277_11b85bdb8e09);
    const IID_FILE_OPERATION: GUID = GUID::from_u128(0x947aab5f_0a5c_4c13_b4d6_4bf7836fc9f8);
    const IID_SHELL_ITEM: GUID = GUID::from_u128(0x43826d1e_e718_42ee_bc55_a1e261c37bfe);
    const IID_PROGRESS_SINK: GUID = GUID::from_u128(0x04b0f1a7_9490_44bc_96e1_4296a31252e2);
    const IID_UNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);

    const S_OK: HRESULT = 0;
    const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as HRESULT;
    const E_ABORT: HRESULT = 0x8000_4004_u32 as HRESULT;
    const E_FAIL: HRESULT = 0x8000_4005_u32 as HRESULT;

    /// Aucune interface (`FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI |
    /// FOF_NOCONFIRMMKDIR`).
    const FOF_NO_UI: u32 = 0x0614;
    const FOF_ALLOWUNDO: u32 = 0x0040;
    /// Mettre à la corbeille plutôt que supprimer (Windows 8 et suivants).
    const FOFX_RECYCLEONDELETE: u32 = 0x0008_0000;
    /// S'arrêter au premier échec.
    const FOFX_EARLYFAILURE: u32 = 0x0010_0000;
    /// Posé par Windows sur un élément qu'il va mettre à la corbeille.
    const TSF_DELETE_RECYCLE_IF_POSSIBLE: u32 = 0x80;

    /// Codes du moteur de copie qui signifient « la corbeille ne peut pas le
    /// recevoir » (`COPYENGINE_E_RECYCLE_*`).
    const RECYCLE_ERRORS: [u32; 5] = [
        0x8027_0035, // UNKNOWN_ERROR
        0x8027_0036, // FORCE_NUKE
        0x8027_0037, // SIZE_TOO_BIG
        0x8027_0038, // PATH_TOO_LONG
        0x8027_003A, // BIN_NOT_FOUND
    ];

    type This = *mut c_void;

    #[repr(C)]
    struct UnknownVtbl {
        query_interface: unsafe extern "system" fn(This, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(This) -> u32,
        release: unsafe extern "system" fn(This) -> u32,
    }

    #[repr(C)]
    struct FileOperationVtbl {
        base: UnknownVtbl,
        advise: usize,
        unadvise: usize,
        set_operation_flags: unsafe extern "system" fn(This, u32) -> HRESULT,
        set_progress_message: usize,
        set_progress_dialog: usize,
        set_properties: usize,
        set_owner_window: usize,
        apply_properties_to_item: usize,
        apply_properties_to_items: usize,
        rename_item: usize,
        rename_items: usize,
        move_item: usize,
        move_items: usize,
        copy_item: usize,
        copy_items: usize,
        delete_item: unsafe extern "system" fn(This, This, This) -> HRESULT,
        delete_items: usize,
        new_item: usize,
        perform_operations: unsafe extern "system" fn(This) -> HRESULT,
        get_any_operations_aborted: unsafe extern "system" fn(This, *mut i32) -> HRESULT,
    }

    #[repr(C)]
    struct SinkVtbl {
        base: UnknownVtbl,
        start_operations: unsafe extern "system" fn(This) -> HRESULT,
        finish_operations: unsafe extern "system" fn(This, HRESULT) -> HRESULT,
        pre_rename_item: unsafe extern "system" fn(This, u32, This, PCWSTR) -> HRESULT,
        post_rename_item:
            unsafe extern "system" fn(This, u32, This, PCWSTR, HRESULT, This) -> HRESULT,
        pre_move_item: unsafe extern "system" fn(This, u32, This, This, PCWSTR) -> HRESULT,
        post_move_item:
            unsafe extern "system" fn(This, u32, This, This, PCWSTR, HRESULT, This) -> HRESULT,
        pre_copy_item: unsafe extern "system" fn(This, u32, This, This, PCWSTR) -> HRESULT,
        post_copy_item:
            unsafe extern "system" fn(This, u32, This, This, PCWSTR, HRESULT, This) -> HRESULT,
        pre_delete_item: unsafe extern "system" fn(This, u32, This) -> HRESULT,
        post_delete_item: unsafe extern "system" fn(This, u32, This, HRESULT, This) -> HRESULT,
        pre_new_item: unsafe extern "system" fn(This, u32, This, PCWSTR) -> HRESULT,
        post_new_item: unsafe extern "system" fn(
            This,
            u32,
            This,
            PCWSTR,
            PCWSTR,
            u32,
            HRESULT,
            This,
        ) -> HRESULT,
        update_progress: unsafe extern "system" fn(This, u32, u32) -> HRESULT,
        reset_timer: unsafe extern "system" fn(This) -> HRESULT,
        pause_timer: unsafe extern "system" fn(This) -> HRESULT,
        resume_timer: unsafe extern "system" fn(This) -> HRESULT,
    }

    /// Observateur des suppressions. Vit sur la pile de [`run`], plus longtemps
    /// que l'opération qui le consulte : son compteur de références est factice.
    #[repr(C)]
    struct Sink {
        vtbl: *const SinkVtbl,
        /// Windows n'annonçait pas une mise à la corbeille : suppression refusée.
        refused: Cell<bool>,
        /// Premier échec rapporté pour un élément.
        failure: Cell<HRESULT>,
    }

    static SINK_VTBL: SinkVtbl = SinkVtbl {
        base: UnknownVtbl {
            query_interface: sink_query_interface,
            add_ref: sink_ref,
            release: sink_ref,
        },
        start_operations: sink_ok,
        finish_operations: sink_finish,
        pre_rename_item: sink_pre_named,
        post_rename_item: sink_post_rename,
        pre_move_item: sink_pre_transfer,
        post_move_item: sink_post_transfer,
        pre_copy_item: sink_pre_transfer,
        post_copy_item: sink_post_transfer,
        pre_delete_item: sink_pre_delete,
        post_delete_item: sink_post_delete,
        pre_new_item: sink_pre_named,
        post_new_item: sink_post_new,
        update_progress: sink_progress,
        reset_timer: sink_ok,
        pause_timer: sink_ok,
        resume_timer: sink_ok,
    };

    fn same_guid(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    unsafe extern "system" fn sink_query_interface(
        this: This,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> HRESULT {
        if out.is_null() || iid.is_null() {
            return E_FAIL;
        }
        // SAFETY: `iid` et `out` sont des pointeurs valides fournis par COM.
        unsafe {
            if same_guid(&*iid, &IID_UNKNOWN) || same_guid(&*iid, &IID_PROGRESS_SINK) {
                *out = this;
                S_OK
            } else {
                *out = ptr::null_mut();
                E_NOINTERFACE
            }
        }
    }

    unsafe extern "system" fn sink_ref(_this: This) -> u32 {
        1
    }

    unsafe extern "system" fn sink_ok(_this: This) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_finish(_this: This, _result: HRESULT) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_pre_named(
        _this: This,
        _flags: u32,
        _item: This,
        _name: PCWSTR,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_post_rename(
        _this: This,
        _flags: u32,
        _item: This,
        _name: PCWSTR,
        _result: HRESULT,
        _created: This,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_pre_transfer(
        _this: This,
        _flags: u32,
        _item: This,
        _destination: This,
        _name: PCWSTR,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_post_transfer(
        _this: This,
        _flags: u32,
        _item: This,
        _destination: This,
        _name: PCWSTR,
        _result: HRESULT,
        _created: This,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_pre_delete(this: This, flags: u32, _item: This) -> HRESULT {
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE != 0 {
            return S_OK;
        }
        // Windows s'apprête à supprimer pour de bon : on refuse.
        // SAFETY: `this` est le `Sink` que `run` a passé à `DeleteItem`.
        unsafe { (*(this as *const Sink)).refused.set(true) };
        E_ABORT
    }

    unsafe extern "system" fn sink_post_delete(
        this: This,
        _flags: u32,
        _item: This,
        result: HRESULT,
        _created: This,
    ) -> HRESULT {
        if result < 0 {
            // SAFETY: comme dans `sink_pre_delete`.
            let sink = unsafe { &*(this as *const Sink) };
            if sink.failure.get() == S_OK {
                sink.failure.set(result);
            }
        }
        S_OK
    }

    unsafe extern "system" fn sink_post_new(
        _this: This,
        _flags: u32,
        _destination: This,
        _name: PCWSTR,
        _template: PCWSTR,
        _attributes: u32,
        _result: HRESULT,
        _created: This,
    ) -> HRESULT {
        S_OK
    }

    unsafe extern "system" fn sink_progress(_this: This, _total: u32, _done: u32) -> HRESULT {
        S_OK
    }

    /// Pointeur d'interface COM, relâché à la sortie de portée.
    struct ComPtr(This);

    impl ComPtr {
        /// SAFETY (appelant) : `T` est la table de fonctions de cette interface.
        unsafe fn vtbl<T>(&self) -> &T {
            // SAFETY: un pointeur d'interface COM pointe sur un pointeur de table.
            unsafe { &**(self.0 as *const *const T) }
        }
    }

    impl Drop for ComPtr {
        fn drop(&mut self) {
            // SAFETY: toute interface COM commence par les trois fonctions d'IUnknown.
            unsafe { (self.vtbl::<UnknownVtbl>().release)(self.0) };
        }
    }

    struct ComApartment;

    impl Drop for ComApartment {
        fn drop(&mut self) {
            // SAFETY: fait pendant du `CoInitializeEx` réussi de `run`, même fil.
            unsafe { CoUninitialize() };
        }
    }

    /// L'opération tourne dans un fil à elle : son appartement COM est neuf quel
    /// que soit l'état du fil appelant (tâche de fond quelconque).
    pub(super) fn recycle(paths: &[PathBuf]) -> Outcome {
        std::thread::scope(|scope| {
            scope
                .spawn(|| run(paths))
                .join()
                .unwrap_or(Outcome::Failed(E_FAIL))
        })
    }

    fn wide(path: &std::path::Path) -> Vec<u16> {
        // Le shell n'accepte que des barres obliques inverses.
        path.as_os_str()
            .encode_wide()
            .map(|unit| {
                if unit == u16::from(b'/') {
                    u16::from(b'\\')
                } else {
                    unit
                }
            })
            .chain(std::iter::once(0))
            .collect()
    }

    fn run(paths: &[PathBuf]) -> Outcome {
        // SAFETY: appels COM classiques ; chaque pointeur reçu est vérifié avant
        // usage et relâché par `ComPtr`. Les tables de fonctions déclarées plus
        // haut reprennent l'ordre et les signatures des interfaces du shell.
        unsafe {
            let hr = CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            );
            if hr < 0 {
                return Outcome::Failed(hr);
            }
            let _apartment = ComApartment;

            let mut raw: This = ptr::null_mut();
            let hr = CoCreateInstance(
                &CLSID_FILE_OPERATION,
                ptr::null_mut(),
                CLSCTX_ALL,
                &IID_FILE_OPERATION,
                &mut raw,
            );
            if hr < 0 || raw.is_null() {
                return Outcome::Failed(if hr < 0 { hr } else { E_FAIL });
            }
            let operation = ComPtr(raw);
            let vtbl = operation.vtbl::<FileOperationVtbl>();

            let hr = (vtbl.set_operation_flags)(
                operation.0,
                FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE,
            );
            if hr < 0 {
                return Outcome::Failed(hr);
            }

            let sink = Sink {
                vtbl: &SINK_VTBL,
                refused: Cell::new(false),
                failure: Cell::new(S_OK),
            };
            let sink_ptr = &sink as *const Sink as This;

            for path in paths {
                let name = wide(path);
                let mut raw_item: This = ptr::null_mut();
                let hr = SHCreateItemFromParsingName(
                    name.as_ptr(),
                    ptr::null_mut(),
                    &IID_SHELL_ITEM,
                    &mut raw_item,
                );
                if hr < 0 || raw_item.is_null() {
                    return Outcome::Failed(if hr < 0 { hr } else { E_FAIL });
                }
                let item = ComPtr(raw_item);
                let hr = (vtbl.delete_item)(operation.0, item.0, sink_ptr);
                if hr < 0 {
                    return Outcome::Failed(hr);
                }
            }

            let performed = (vtbl.perform_operations)(operation.0);
            let mut aborted = 0i32;
            let _ = (vtbl.get_any_operations_aborted)(operation.0, &mut aborted);
            // L'opération (et ce qu'elle retient de l'observateur) disparaît avant
            // l'observateur lui-même.
            drop(operation);

            let failure = sink.failure.get();
            let is_recycle_error = |code: HRESULT| RECYCLE_ERRORS.contains(&(code as u32));
            if sink.refused.get() || is_recycle_error(failure) || is_recycle_error(performed) {
                Outcome::Unavailable
            } else if failure < 0 {
                Outcome::Failed(failure)
            } else if performed < 0 {
                Outcome::Failed(performed)
            } else if aborted != 0 {
                Outcome::Failed(E_ABORT)
            } else {
                Outcome::Done
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_missing_paths_are_refused_before_anything_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let kept = dir.path().join("gardé.txt");
        fs::write(&kept, "x").unwrap();

        assert!(matches!(
            move_to_trash(&[PathBuf::from("relatif.txt")]),
            Err(Error::Invalid(_))
        ));
        let missing = dir.path().join("absent.txt");
        let error = move_to_trash(&[kept.clone(), missing.clone()]).unwrap_err();
        assert!(matches!(&error, Error::Io { path, .. } if path == &missing));
        assert!(kept.exists(), "rien n'est supprimé si un chemin manque");

        move_to_trash::<PathBuf>(&[]).unwrap();
    }

    /// Envoie de vrais fichiers dans la corbeille de la machine : lancé à la
    /// demande seulement (`cargo test -p julie-core -- --ignored corbeille`).
    #[cfg(windows)]
    #[test]
    #[ignore = "remplit la vraie corbeille Windows"]
    fn corbeille_files_and_folders_really_go_to_the_recycle_bin() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("juliestorm-essai été (1).txt");
        fs::write(&file, "essai de corbeille JulieStorm").unwrap();
        let folder = dir.path().join("juliestorm-essai dossier à jeter");
        fs::create_dir_all(folder.join("sous dossier")).unwrap();
        fs::write(folder.join("sous dossier").join("a.php"), "<?php\n").unwrap();
        fs::write(folder.join("b.txt"), "b").unwrap();
        let kept = dir.path().join("juliestorm-essai gardé.txt");
        fs::write(&kept, "reste").unwrap();
        // Barres obliques dans le chemin : acceptées.
        let slashed = PathBuf::from(file.to_string_lossy().replace('\\', "/"));

        move_to_trash(&[slashed, folder.clone()]).unwrap();
        assert!(!file.exists());
        assert!(!folder.exists());
        assert!(kept.exists());
    }

    /// Un fichier tenu ouvert sans partage par un autre programme : il reste sur
    /// le disque et l'erreur le nomme.
    #[cfg(windows)]
    #[test]
    #[ignore = "remplit la vraie corbeille Windows"]
    fn corbeille_a_locked_file_stays_on_disk_and_is_reported() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("juliestorm-essai verrouillé.txt");
        fs::write(&locked, "tenu").unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .unwrap();

        let error = move_to_trash(std::slice::from_ref(&locked)).unwrap_err();
        match &error {
            Error::TrashFailed { remaining, .. } => {
                assert_eq!(remaining.as_slice(), std::slice::from_ref(&locked))
            }
            other => panic!("erreur inattendue : {other:?}"),
        }
        drop(handle);
        assert_eq!(fs::read_to_string(&locked).unwrap(), "tenu");
    }

    /// Un partage réseau n'a pas de corbeille : rien ne doit être supprimé. Passe
    /// par le partage administratif `\\localhost\C$` ; sans accès, l'essai le dit
    /// et ne conclut rien.
    #[cfg(windows)]
    #[test]
    #[ignore = "a besoin du partage administratif \\\\localhost\\C$"]
    fn corbeille_nothing_is_deleted_when_there_is_no_recycle_bin() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("juliestorm-essai réseau.txt");
        fs::write(&file, "ne doit pas disparaître").unwrap();
        let local = file.to_string_lossy().into_owned();
        let Some(rest) = local.strip_prefix("C:\\") else {
            eprintln!("NON CONCLUANT : dossier temporaire hors de C:");
            return;
        };
        let network = PathBuf::from(format!("\\\\localhost\\C$\\{rest}"));
        if fs::symlink_metadata(&network).is_err() {
            eprintln!("NON CONCLUANT : {} inaccessible", network.display());
            return;
        }

        let error = move_to_trash(std::slice::from_ref(&network)).unwrap_err();
        eprintln!("erreur rendue : {error:?}");
        assert!(file.exists(), "le fichier a été supprimé définitivement");
        assert!(matches!(error, Error::TrashUnavailable { .. }), "{error:?}");
    }
}
