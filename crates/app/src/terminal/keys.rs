//! Traduction des événements clavier d'egui en actions sur une session.
//! Fonction pure : testée sans fenêtre.

use eframe::egui::{Event, ImeEvent, Key as EguiKey, Modifiers as EguiModifiers};
use julie_terminal::{Key, Modifiers, ScrollAction};

/// Ce qu'un événement demande au terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Text(String),
    Key(Key, Modifiers),
    Paste(String),
    /// Copier la sélection dans le presse-papiers.
    Copy,
    /// Demander le contenu du presse-papiers (arrive en `Event::Paste`).
    RequestPaste,
    Scroll(ScrollAction),
}

/// Raccourcis gardés par l'IDE même quand le terminal a le focus
/// (`docs/specs/raccourcis.md`, Règles) : le terminal ne les envoie pas au shell.
pub fn reserved_for_host(key: EguiKey, m: EguiModifiers) -> bool {
    let digit = matches!(
        key,
        EguiKey::Num0
            | EguiKey::Num1
            | EguiKey::Num2
            | EguiKey::Num3
            | EguiKey::Num4
            | EguiKey::Num5
            | EguiKey::Num6
            | EguiKey::Num7
            | EguiKey::Num8
            | EguiKey::Num9
    );
    let alt_only = m.alt && !m.ctrl && !m.shift;
    let ctrl_shift = m.ctrl && m.shift && !m.alt;
    (alt_only && (digit || key == EguiKey::F12))
        || (ctrl_shift && matches!(key, EguiKey::N | EguiKey::F | EguiKey::T))
        || (m.ctrl && !m.shift && !m.alt && key == EguiKey::F4)
}

fn char_of(key: EguiKey) -> Option<char> {
    use EguiKey::*;
    Some(match key {
        A => 'a',
        B => 'b',
        C => 'c',
        D => 'd',
        E => 'e',
        F => 'f',
        G => 'g',
        H => 'h',
        I => 'i',
        J => 'j',
        K => 'k',
        L => 'l',
        M => 'm',
        N => 'n',
        O => 'o',
        P => 'p',
        Q => 'q',
        R => 'r',
        S => 's',
        T => 't',
        U => 'u',
        V => 'v',
        W => 'w',
        X => 'x',
        Y => 'y',
        Z => 'z',
        Num0 => '0',
        Num1 => '1',
        Num2 => '2',
        Num3 => '3',
        Num4 => '4',
        Num5 => '5',
        Num6 => '6',
        Num7 => '7',
        Num8 => '8',
        Num9 => '9',
        Space => ' ',
        OpenBracket => '[',
        CloseBracket => ']',
        Backslash => '\\',
        Slash => '/',
        Minus => '-',
        Period => '.',
        Comma => ',',
        Semicolon => ';',
        Equals => '=',
        Questionmark => '?',
        _ => return None,
    })
}

fn special(key: EguiKey) -> Option<Key> {
    use EguiKey::*;
    Some(match key {
        Enter => Key::Enter,
        Tab => Key::Tab,
        Backspace => Key::Backspace,
        Escape => Key::Escape,
        ArrowUp => Key::Up,
        ArrowDown => Key::Down,
        ArrowLeft => Key::Left,
        ArrowRight => Key::Right,
        Home => Key::Home,
        End => Key::End,
        PageUp => Key::PageUp,
        PageDown => Key::PageDown,
        Insert => Key::Insert,
        Delete => Key::Delete,
        F1 => Key::F(1),
        F2 => Key::F(2),
        F3 => Key::F(3),
        F4 => Key::F(4),
        F5 => Key::F(5),
        F6 => Key::F(6),
        F7 => Key::F(7),
        F8 => Key::F(8),
        F9 => Key::F(9),
        F10 => Key::F(10),
        F11 => Key::F(11),
        F12 => Key::F(12),
        _ => return None,
    })
}

/// État gardé d'un événement à l'autre.
#[derive(Debug, Default, Clone)]
pub struct KeyState {
    /// Alt+lettre vient d'être codé en `ESC lettre` : le texte « lettre » qu'egui
    /// émet juste après ne doit pas partir une seconde fois.
    skip_next_text: bool,
}

/// Actions pour un événement. `has_selection` : Ctrl+C copie au lieu
/// d'interrompre ; `modifiers` : état courant (egui ne le joint pas à
/// `Copy`/`Cut`/`Paste`).
pub fn translate(
    event: &Event,
    modifiers: EguiModifiers,
    has_selection: bool,
    state: &mut KeyState,
) -> Vec<Action> {
    match event {
        Event::Text(text) => {
            if std::mem::take(&mut state.skip_next_text) {
                return Vec::new();
            }
            vec![Action::Text(text.clone())]
        }
        Event::Ime(ImeEvent::Commit(text)) if !text.is_empty() => {
            vec![Action::Text(text.clone())]
        }
        // Ctrl+C (et Ctrl+Inser) : copie s'il y a une sélection, sinon interruption.
        Event::Copy => {
            if has_selection {
                vec![Action::Copy]
            } else if modifiers.ctrl && !modifiers.shift {
                vec![Action::Key(Key::Char('c'), Modifiers::CTRL)]
            } else {
                Vec::new()
            }
        }
        // Ctrl+X va au shell ; Maj+Suppr aussi (egui en fait un « couper »).
        Event::Cut => {
            if modifiers.shift && !modifiers.ctrl {
                vec![Action::Key(Key::Delete, Modifiers::SHIFT)]
            } else {
                vec![Action::Key(Key::Char('x'), Modifiers::CTRL)]
            }
        }
        Event::Paste(text) => vec![Action::Paste(text.clone())],
        Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } => {
            state.skip_next_text = false;
            let key = *key;
            let m = *m;
            if reserved_for_host(key, m) {
                return Vec::new();
            }
            // Ctrl+Maj+C / Ctrl+Maj+V : copier / coller toujours.
            if m.ctrl && m.shift && !m.alt {
                match key {
                    EguiKey::C => {
                        return if has_selection {
                            vec![Action::Copy]
                        } else {
                            Vec::new()
                        }
                    }
                    EguiKey::V => return vec![Action::RequestPaste],
                    _ => {}
                }
            }
            // Maj+Page préc. / suiv., Maj+Origine / Fin : l'historique.
            if m.shift && !m.ctrl && !m.alt {
                match key {
                    EguiKey::PageUp => return vec![Action::Scroll(ScrollAction::PageUp)],
                    EguiKey::PageDown => return vec![Action::Scroll(ScrollAction::PageDown)],
                    _ => {}
                }
            }
            let modifiers = Modifiers {
                shift: m.shift,
                ctrl: m.ctrl,
                alt: m.alt,
            };
            if let Some(special) = special(key) {
                return vec![Action::Key(special, modifiers)];
            }
            // Lettres et ponctuation : seulement avec Ctrl ou Alt (le reste
            // arrive en `Event::Text`). AltGr n'est pas vu comme Ctrl+Alt par
            // winit : le texte d'AltGr arrive en `Event::Text`.
            if m.ctrl || m.alt {
                if let Some(c) = char_of(key) {
                    if m.alt && !m.ctrl {
                        state.skip_next_text = true;
                    }
                    return vec![Action::Key(Key::Char(c), modifiers)];
                }
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: EguiKey, modifiers: EguiModifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn run(events: &[Event], modifiers: EguiModifiers, selection: bool) -> Vec<Action> {
        let mut state = KeyState::default();
        events
            .iter()
            .flat_map(|e| translate(e, modifiers, selection, &mut state))
            .collect()
    }

    const CTRL: EguiModifiers = EguiModifiers::CTRL;
    const NONE: EguiModifiers = EguiModifiers::NONE;

    #[test]
    fn ctrl_c_interrupts_or_copies() {
        assert_eq!(
            run(&[Event::Copy], CTRL, false),
            vec![Action::Key(Key::Char('c'), Modifiers::CTRL)]
        );
        assert_eq!(run(&[Event::Copy], CTRL, true), vec![Action::Copy]);
        let ctrl_shift = EguiModifiers::CTRL | EguiModifiers::SHIFT;
        assert_eq!(
            run(&[key(EguiKey::C, ctrl_shift)], ctrl_shift, true),
            vec![Action::Copy]
        );
        assert!(run(&[key(EguiKey::C, ctrl_shift)], ctrl_shift, false).is_empty());
        assert_eq!(
            run(&[key(EguiKey::V, ctrl_shift)], ctrl_shift, false),
            vec![Action::RequestPaste]
        );
    }

    #[test]
    fn ctrl_d_and_ctrl_r_go_to_the_shell() {
        assert_eq!(
            run(&[key(EguiKey::D, CTRL)], CTRL, false),
            vec![Action::Key(Key::Char('d'), Modifiers::CTRL)]
        );
        assert_eq!(
            run(&[key(EguiKey::R, CTRL)], CTRL, false),
            vec![Action::Key(Key::Char('r'), Modifiers::CTRL)]
        );
    }

    #[test]
    fn escape_tab_arrows_and_function_keys_go_to_the_shell() {
        for (egui_key, expected) in [
            (EguiKey::Escape, Key::Escape),
            (EguiKey::Tab, Key::Tab),
            (EguiKey::ArrowUp, Key::Up),
            (EguiKey::ArrowLeft, Key::Left),
            (EguiKey::F5, Key::F(5)),
            (EguiKey::Enter, Key::Enter),
        ] {
            assert_eq!(
                run(&[key(egui_key, NONE)], NONE, false),
                vec![Action::Key(expected, Modifiers::NONE)]
            );
        }
    }

    #[test]
    fn host_shortcuts_are_left_alone() {
        let alt = EguiModifiers::ALT;
        let ctrl_shift = EguiModifiers::CTRL | EguiModifiers::SHIFT;
        assert!(run(&[key(EguiKey::F12, alt)], alt, false).is_empty());
        assert!(run(&[key(EguiKey::Num1, alt)], alt, false).is_empty());
        assert!(run(&[key(EguiKey::N, ctrl_shift)], ctrl_shift, false).is_empty());
        assert!(run(&[key(EguiKey::T, ctrl_shift)], ctrl_shift, false).is_empty());
        assert!(run(&[key(EguiKey::F4, CTRL)], CTRL, false).is_empty());
    }

    #[test]
    fn text_accents_and_altgr_pass_through() {
        // Touche morte ^ puis e : winit livre « ê » déjà composé.
        assert_eq!(
            run(&[Event::Text("ê".into())], NONE, false),
            vec![Action::Text("ê".into())]
        );
        // AltGr+0 (AZERTY) : winit retire Ctrl+Alt ; la touche seule ne code rien.
        assert_eq!(
            run(
                &[key(EguiKey::Num0, NONE), Event::Text("@".into())],
                NONE,
                false
            ),
            vec![Action::Text("@".into())]
        );
        assert_eq!(
            run(&[Event::Ime(ImeEvent::Commit("日本".into()))], NONE, false),
            vec![Action::Text("日本".into())]
        );
    }

    #[test]
    fn alt_letter_is_sent_once_as_escape_prefix() {
        let alt = EguiModifiers::ALT;
        assert_eq!(
            run(&[key(EguiKey::B, alt), Event::Text("b".into())], alt, false),
            vec![Action::Key(Key::Char('b'), Modifiers::ALT)]
        );
    }

    #[test]
    fn paste_and_shift_page_scroll() {
        assert_eq!(
            run(&[Event::Paste("ls\n".into())], CTRL, false),
            vec![Action::Paste("ls\n".into())]
        );
        let shift = EguiModifiers::SHIFT;
        assert_eq!(
            run(&[key(EguiKey::PageUp, shift)], shift, false),
            vec![Action::Scroll(ScrollAction::PageUp)]
        );
        assert_eq!(
            run(&[Event::Cut], CTRL, false),
            vec![Action::Key(Key::Char('x'), Modifiers::CTRL)]
        );
    }
}
