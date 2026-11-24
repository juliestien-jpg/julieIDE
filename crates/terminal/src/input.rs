//! Codage de la saisie en octets pour le programme (séquences xterm).
//!
//! Le texte ordinaire (lettres, accents déjà composés par la disposition du
//! clavier, AltGr) s'envoie tel quel en UTF-8 par `Session::send_text` ; ce
//! module ne code que les touches spéciales et les combinaisons Ctrl / Alt.

use crate::screen::Modes;

/// Une touche, indépendamment de la bibliothèque d'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// Un caractère tapé avec Ctrl et/ou Alt (lettre, chiffre, ponctuation).
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// Touche de fonction F1 à F12.
    F(u8),
}

/// Touches de modification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const CTRL: Modifiers = Modifiers {
        shift: false,
        ctrl: true,
        alt: false,
    };
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ctrl: false,
        alt: false,
    };
    pub const ALT: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: true,
    };

    /// Paramètre xterm des modificateurs (`1 + shift + 2·alt + 4·ctrl`) ; 1 = aucun.
    fn xterm_parameter(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }

    fn any(self) -> bool {
        self.shift || self.ctrl || self.alt
    }
}

/// Octet de contrôle de Ctrl+`c`, comme xterm (`Ctrl+C` → 0x03, `Ctrl+[` → Échap…).
pub fn control_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// Octets à envoyer pour une touche ; `None` si elle n'a pas de codage
/// (l'hôte peut alors la garder pour lui).
pub fn encode_key(key: Key, modifiers: Modifiers, modes: &Modes) -> Option<Vec<u8>> {
    let m = modifiers;
    let csi_mod = |final_byte: u8| -> Vec<u8> {
        if m.any() {
            format!("\x1b[1;{}{}", m.xterm_parameter(), final_byte as char).into_bytes()
        } else if modes.app_cursor {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if m.any() {
            format!("\x1b[{code};{}~", m.xterm_parameter()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let alt_prefix = |mut bytes: Vec<u8>| -> Vec<u8> {
        if m.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };

    Some(match key {
        Key::Char(c) => {
            if m.ctrl {
                alt_prefix(vec![control_byte(c)?])
            } else if m.alt {
                let mut bytes = vec![0x1b];
                let c = if m.shift { c.to_ascii_uppercase() } else { c };
                let mut utf8 = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
                bytes
            } else {
                // Sans Ctrl ni Alt, c'est du texte : `send_text`.
                return None;
            }
        }
        Key::Enter => alt_prefix(vec![b'\r']),
        Key::Tab if m.shift => b"\x1b[Z".to_vec(),
        Key::Tab => alt_prefix(vec![b'\t']),
        Key::Backspace if m.ctrl => alt_prefix(vec![0x08]),
        Key::Backspace => alt_prefix(vec![0x7f]),
        Key::Escape => alt_prefix(vec![0x1b]),
        Key::Up => csi_mod(b'A'),
        Key::Down => csi_mod(b'B'),
        Key::Right => csi_mod(b'C'),
        Key::Left => csi_mod(b'D'),
        Key::Home => csi_mod(b'H'),
        Key::End => csi_mod(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let final_byte = b'P' + (n - 1);
            if m.any() {
                format!("\x1b[1;{}{}", m.xterm_parameter(), final_byte as char).into_bytes()
            } else {
                vec![0x1b, b'O', final_byte]
            }
        }
        Key::F(n @ 5..=12) => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            tilde(CODES[(n - 5) as usize])
        }
        Key::F(_) => return None,
    })
}

/// Octets d'un collage. Les fins de ligne deviennent des retours chariot
/// (ce qu'envoie la touche Entrée). En mode collage encadré, le texte est
/// entouré de `ESC [200~` … `ESC [201~` et ne peut pas fermer l'encadrement
/// lui-même (les `ESC` du texte sont retirés).
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if !bracketed {
        return normalized.into_bytes();
    }
    let mut bytes = b"\x1b[200~".to_vec();
    bytes.extend(normalized.bytes().filter(|&b| b != 0x1b));
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, m: Modifiers) -> Vec<u8> {
        encode_key(key, m, &Modes::default()).unwrap()
    }

    #[test]
    fn control_characters() {
        assert_eq!(key(Key::Char('c'), Modifiers::CTRL), vec![0x03]);
        assert_eq!(key(Key::Char('D'), Modifiers::CTRL), vec![0x04]);
        assert_eq!(key(Key::Char('r'), Modifiers::CTRL), vec![0x12]);
        assert_eq!(key(Key::Char('['), Modifiers::CTRL), vec![0x1b]);
        assert_eq!(key(Key::Char(' '), Modifiers::CTRL), vec![0x00]);
        assert_eq!(
            encode_key(Key::Char('é'), Modifiers::CTRL, &Modes::default()),
            None
        );
        assert_eq!(
            encode_key(Key::Char('a'), Modifiers::NONE, &Modes::default()),
            None
        );
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(key(Key::Char('b'), Modifiers::ALT), b"\x1bb".to_vec());
        let alt_shift = Modifiers {
            shift: true,
            alt: true,
            ctrl: false,
        };
        assert_eq!(key(Key::Char('b'), alt_shift), b"\x1bB".to_vec());
        assert_eq!(key(Key::Backspace, Modifiers::ALT), b"\x1b\x7f".to_vec());
    }

    #[test]
    fn special_keys() {
        assert_eq!(key(Key::Enter, Modifiers::NONE), b"\r".to_vec());
        assert_eq!(key(Key::Tab, Modifiers::NONE), b"\t".to_vec());
        assert_eq!(key(Key::Tab, Modifiers::SHIFT), b"\x1b[Z".to_vec());
        assert_eq!(key(Key::Backspace, Modifiers::NONE), vec![0x7f]);
        assert_eq!(key(Key::Escape, Modifiers::NONE), vec![0x1b]);
        assert_eq!(key(Key::Delete, Modifiers::NONE), b"\x1b[3~".to_vec());
        assert_eq!(key(Key::PageUp, Modifiers::NONE), b"\x1b[5~".to_vec());
        assert_eq!(key(Key::Home, Modifiers::NONE), b"\x1b[H".to_vec());
    }

    #[test]
    fn arrows_normal_application_and_modified() {
        assert_eq!(key(Key::Up, Modifiers::NONE), b"\x1b[A".to_vec());
        let app = Modes {
            app_cursor: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_key(Key::Up, Modifiers::NONE, &app).unwrap(),
            b"\x1bOA".to_vec()
        );
        assert_eq!(key(Key::Left, Modifiers::CTRL), b"\x1b[1;5D".to_vec());
        assert_eq!(key(Key::Right, Modifiers::SHIFT), b"\x1b[1;2C".to_vec());
    }

    #[test]
    fn function_keys() {
        assert_eq!(key(Key::F(1), Modifiers::NONE), b"\x1bOP".to_vec());
        assert_eq!(key(Key::F(4), Modifiers::NONE), b"\x1bOS".to_vec());
        assert_eq!(key(Key::F(5), Modifiers::NONE), b"\x1b[15~".to_vec());
        assert_eq!(key(Key::F(12), Modifiers::NONE), b"\x1b[24~".to_vec());
        assert_eq!(key(Key::F(2), Modifiers::SHIFT), b"\x1b[1;2Q".to_vec());
        assert_eq!(key(Key::F(10), Modifiers::CTRL), b"\x1b[21;5~".to_vec());
        assert_eq!(
            encode_key(Key::F(13), Modifiers::NONE, &Modes::default()),
            None
        );
    }

    #[test]
    fn paste_plain_and_bracketed() {
        assert_eq!(encode_paste("a\r\nb\nc", false), b"a\rb\rc".to_vec());
        assert_eq!(
            encode_paste("ls\n", true),
            b"\x1b[200~ls\r\x1b[201~".to_vec()
        );
        // Le texte ne peut pas fermer l'encadrement.
        assert_eq!(
            encode_paste("x\x1b[201~rm", true),
            b"\x1b[200~x[201~rm\x1b[201~".to_vec()
        );
        assert_eq!(encode_paste("é", false), "é".as_bytes().to_vec());
    }
}
