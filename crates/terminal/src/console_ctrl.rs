//! Ctrl+C dans les pseudo-consoles quand l'IDE a hérité de « ignorer Ctrl+C ».
//!
//! Windows garde dans les paramètres de chaque processus un drapeau « ignorer
//! Ctrl+C » (`RTL_USER_PROCESS_PARAMETERS.ConsoleFlags`, bit 0). Il est posé par
//! `SetConsoleCtrlHandler(NULL, TRUE)` ou, implicitement, quand un parent lance
//! le processus avec `CREATE_NEW_PROCESS_GROUP` ; et **chaque enfant en hérite à
//! sa création**, pseudo-console ou pas, sans drapeau de création pour l'éviter.
//! Quand il est posé, `CTRL_C_EVENT` n'appelle aucun gestionnaire : le processus
//! ne fait rien.
//!
//! Chaîne observée (écart M2 de la recette) : le Git Bash de l'outil d'agent
//! tourne avec le drapeau posé (lu : vrai dans un `cargo test` lancé de là,
//! faux depuis PowerShell) → cargo → l'IDE ou l'exécutable de test (hérité) →
//! `cmd.exe` dans la pseudo-console (hérité) → `PING` (hérité). conhost envoie
//! bien `CTRL_C_EVENT` en recevant `0x03`, mais cmd et ping l'ignorent. Git
//! Bash n'y est pas sensible (vérifié) : le runtime MSYS traite `^C` lui-même.
//!
//! Correctif (le même que celui de Windows Terminal, qui a le même problème) :
//! avant le premier lancement, si notre processus ignore Ctrl+C, on remet le
//! drapeau à zéro (`SetConsoleCtrlHandler(NULL, FALSE)`) pour que les enfants
//! naissent dans l'état normal. Pour que **l'IDE lui-même** continue d'ignorer
//! Ctrl+C (s'il a une console, en version de mise au point), on installe
//! d'abord un gestionnaire qui avale `CTRL_C_EVENT` et seulement lui
//! (Ctrl+Pause, fermeture de la console… gardent leur effet par défaut). Les
//! gestionnaires ne s'héritent pas, le drapeau si : les enfants retrouvent le
//! comportement habituel. Si le processus n'ignorait pas Ctrl+C, rien n'est
//! touché.
//!
//! Lire le drapeau passe par le bloc d'environnement du processus (PEB) : le
//! champ n'est pas documenté par Microsoft, mais stable depuis Windows NT
//! (ReactOS et Wine le décrivent à la même place). Si la lecture échoue, on ne
//! touche à rien.

use std::sync::Once;

use windows_sys::core::BOOL;
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, PROCESS_BASIC_INFORMATION};

/// Bit 0 de `ConsoleFlags` : « ignorer Ctrl+C ».
const CONSOLE_IGNORE_CTRL_C: u32 = 1;

static PREPARE: Once = Once::new();

/// Le processus courant ignore-t-il Ctrl+C (drapeau hérité ou posé) ?
/// `None` si l'information n'a pas pu être lue.
pub(crate) fn ctrl_c_ignored() -> Option<bool> {
    // SAFETY : structure C, zéro valide ; tampon de la bonne taille pour
    // ProcessBasicInformation ; pseudo-poignée du processus courant.
    unsafe {
        let mut info: PROCESS_BASIC_INFORMATION = std::mem::zeroed();
        let status = NtQueryInformationProcess(
            GetCurrentProcess(),
            ProcessBasicInformation,
            &mut info as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            std::ptr::null_mut(),
        );
        if status < 0 || info.PebBaseAddress.is_null() {
            return None;
        }
        let params = std::ptr::read_volatile(&(*info.PebBaseAddress).ProcessParameters);
        if params.is_null() {
            return None;
        }
        // Reserved2[0] = ConsoleHandle, Reserved2[1] = ConsoleFlags (un ULONG,
        // suivi d'un remplissage en 64 bits : on ne garde que les 32 bits bas).
        let flags = std::ptr::read_volatile(&(*params).Reserved2[1]) as usize as u32;
        Some(flags & CONSOLE_IGNORE_CTRL_C != 0)
    }
}

/// Avale Ctrl+C pour l'IDE ; laisse passer tout le reste.
unsafe extern "system" fn swallow_ctrl_c(ctrl: u32) -> BOOL {
    BOOL::from(ctrl == CTRL_C_EVENT)
}

/// À appeler avant de lancer un interpréteur : les enfants à venir ne
/// naîtront pas en ignorant Ctrl+C (voir la documentation du module). Une
/// seule fois par processus ; sans effet si le processus n'ignorait pas Ctrl+C.
pub(crate) fn let_children_receive_ctrl_c() {
    PREPARE.call_once(|| {
        if ctrl_c_ignored() != Some(true) {
            return;
        }
        // SAFETY : gestionnaire `extern "system"` valide pour toute la vie du
        // processus ; `None` + FALSE remet le drapeau du processus à zéro.
        unsafe {
            // D'abord notre gestionnaire : pas un instant où un Ctrl+C reçu
            // par l'IDE le terminerait.
            if SetConsoleCtrlHandler(Some(swallow_ctrl_c), 1) == 0 {
                return;
            }
            SetConsoleCtrlHandler(None, 0);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_can_be_read() {
        assert!(ctrl_c_ignored().is_some());
    }

    #[test]
    fn the_handler_swallows_only_ctrl_c() {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT};
        // SAFETY : fonction pure.
        unsafe {
            assert_eq!(swallow_ctrl_c(CTRL_C_EVENT), 1);
            assert_eq!(swallow_ctrl_c(CTRL_BREAK_EVENT), 0);
            assert_eq!(swallow_ctrl_c(CTRL_CLOSE_EVENT), 0);
        }
    }

    #[test]
    fn after_preparing_children_no_longer_inherit_the_flag() {
        let_children_receive_ctrl_c();
        assert_eq!(ctrl_c_ignored(), Some(false));
    }
}
