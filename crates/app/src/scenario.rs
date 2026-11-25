//! Mode scénario : `juliestorm --scenario <fichier>` (ou la variable
//! `JULIESTORM_SCENARIO`) joue une liste d'étapes, une par ligne, dans la vraie
//! fenêtre : ouvrir un projet, lancer une action par identifiant, saisir du
//! texte, appuyer sur une touche, attendre, capturer la fenêtre en PNG, écrire
//! l'état de l'IDE en JSON, quitter. Format et exemples : `docs/essais.md`.
//!
//! Les agents ne peuvent ni cliquer ni taper dans une fenêtre egui : ce mode est
//! leur seul moyen de faire fonctionner l'application et de la regarder.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2};

/// Une étape a échoué (action inconnue, touche illisible…) : code de sortie 2.
pub static FAILED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// `project <dossier>` (attend la fin des tâches de fond) ou
    /// `project-nowait <dossier>`.
    Project { path: PathBuf, wait: bool },
    /// `fullscreen on|off`
    Fullscreen(bool),
    /// `file <chemin>[:ligne[:colonne]]` (relatif à la racine du projet)
    File {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
    /// `action <identifiant>`
    Action(String),
    /// `type <texte>` : dans le champ qui a le focus.
    Type(String),
    /// `key <Ctrl+Maj+N | Enter | Down | …>`
    Key(Combo),
    /// `click <x> <y>` en points de la fenêtre.
    Click(f32, f32),
    /// `rclick <x> <y>` : clic droit (menus contextuels).
    RClick(f32, f32),
    /// `wait <ms>`
    Wait(u64),
    /// `wait-idle` : plus de tâche de fond, index prêt, résultats arrivés.
    WaitIdle,
    /// `screenshot <fichier.png>`
    Screenshot(PathBuf),
    /// `state <fichier.json>`
    State(PathBuf),
    /// `resize <largeur> <hauteur>` (taille intérieure, en points)
    Resize(f32, f32),
    /// `scale <facteur>` : 1.0 = 100 %, 1.5 = 150 %.
    Scale(f32),
    /// `theme dark|light`
    Theme(bool),
    /// `tree-expand <chemin relatif>`
    TreeExpand(String),
    /// `log <texte>`
    Log(String),
    /// `external-write <chemin> <texte>` : réécrit un fichier hors de l'IDE
    /// (la séquence antislash-n fait un saut de ligne) pour simuler un
    /// changement externe. Refusé hors du
    /// dossier temporaire du système.
    ExternalWrite { path: PathBuf, text: String },
    /// `ui <commande> [argument]` : piloter les vues Git, la fusion et les tests.
    Ui { command: String, arg: String },
    /// `quit`
    Quit,
}

/// Une touche et ses modificateurs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Combo {
    pub key: Key,
    pub modifiers: Modifiers,
}

impl Combo {
    /// Les événements à injecter : modificateurs, appui, relâchement.
    pub fn events(&self) -> Vec<Event> {
        let mut events = vec![Event::ModifiersChanged(self.modifiers)];
        events.extend([true, false].into_iter().map(|pressed| Event::Key {
            key: self.key,
            physical_key: Some(self.key),
            pressed,
            repeat: false,
            modifiers: self.modifiers,
        }));
        events.push(Event::ModifiersChanged(Modifiers::NONE));
        events
    }
}

pub fn parse_combo(text: &str) -> Result<Combo, String> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+') {
        match part.trim().to_lowercase().as_str() {
            "ctrl" | "ctl" | "control" => modifiers.ctrl = true,
            "alt" => modifiers.alt = true,
            "shift" | "maj" => modifiers.shift = true,
            "" => return Err(format!("touche illisible : « {text} »")),
            other => {
                let name = match other {
                    "entrée" | "entree" | "return" => "Enter".to_owned(),
                    "échap" | "echap" => "Escape".to_owned(),
                    "gauche" => "Left".to_owned(),
                    "droite" => "Right".to_owned(),
                    "haut" => "Up".to_owned(),
                    "bas" => "Down".to_owned(),
                    "space" | "espace" => "Space".to_owned(),
                    other if other.len() == 1 => other.to_uppercase(),
                    other
                        if other.starts_with('f')
                            && other[1..].chars().all(|c| c.is_ascii_digit()) =>
                    {
                        other.to_uppercase()
                    }
                    other => {
                        let mut chars = other.chars();
                        match chars.next() {
                            Some(first) => {
                                first.to_uppercase().collect::<String>() + chars.as_str()
                            }
                            None => String::new(),
                        }
                    }
                };
                key = Some(
                    Key::from_name(&name).ok_or_else(|| format!("touche inconnue : « {part} »"))?,
                );
            }
        }
    }
    // Hors macOS, `command` est le Ctrl du système.
    modifiers.command = modifiers.ctrl;
    key.map(|key| Combo { key, modifiers })
        .ok_or_else(|| format!("aucune touche dans « {text} »"))
}

/// Lit un scénario : une étape par ligne, `#` commence un commentaire.
pub fn parse(text: &str) -> Result<Vec<(usize, Step)>, String> {
    let mut steps = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = match line.split_once(char::is_whitespace) {
            Some((word, rest)) => (word, rest.trim()),
            None => (line, ""),
        };
        let fail = |message: &str| format!("ligne {number} : {message}");
        let number_arg = |text: &str| -> Result<f32, String> {
            text.parse::<f32>()
                .map_err(|_| fail(&format!("nombre illisible : « {text} »")))
        };
        let step = match word {
            "project" if !rest.is_empty() => Step::Project {
                path: PathBuf::from(rest),
                wait: true,
            },
            "project-nowait" if !rest.is_empty() => Step::Project {
                path: PathBuf::from(rest),
                wait: false,
            },
            "fullscreen" => match rest {
                "on" => Step::Fullscreen(true),
                "off" => Step::Fullscreen(false),
                _ => return Err(fail("`fullscreen on` ou `fullscreen off`")),
            },
            "file" if !rest.is_empty() => {
                // `chemin:ligne:colonne` ; un `C:` de lecteur n'est pas un suffixe.
                let mut path = rest.to_owned();
                let mut numbers = Vec::new();
                while let Some((head, tail)) = path.rsplit_once(':') {
                    match tail.parse::<u32>() {
                        Ok(n) if numbers.len() < 2 && !head.is_empty() => {
                            numbers.push(n);
                            path = head.to_owned();
                        }
                        _ => break,
                    }
                }
                numbers.reverse();
                Step::File {
                    path: PathBuf::from(path),
                    line: numbers.first().copied(),
                    column: numbers.get(1).copied(),
                }
            }
            "action" if !rest.is_empty() => Step::Action(rest.to_owned()),
            "type" => Step::Type(rest.to_owned()),
            "key" if !rest.is_empty() => Step::Key(parse_combo(rest).map_err(|e| fail(&e))?),
            "click" => {
                let mut parts = rest.split_whitespace();
                let x = number_arg(parts.next().unwrap_or(""))?;
                let y = number_arg(parts.next().unwrap_or(""))?;
                Step::Click(x, y)
            }
            "rclick" => {
                let mut parts = rest.split_whitespace();
                let x = number_arg(parts.next().unwrap_or(""))?;
                let y = number_arg(parts.next().unwrap_or(""))?;
                Step::RClick(x, y)
            }
            "wait" => Step::Wait(
                rest.parse::<u64>()
                    .map_err(|_| fail("durée en millisecondes attendue"))?,
            ),
            "wait-idle" => Step::WaitIdle,
            "screenshot" if !rest.is_empty() => Step::Screenshot(PathBuf::from(rest)),
            "state" if !rest.is_empty() => Step::State(PathBuf::from(rest)),
            "resize" => {
                let mut parts = rest.split_whitespace();
                let w = number_arg(parts.next().unwrap_or(""))?;
                let h = number_arg(parts.next().unwrap_or(""))?;
                Step::Resize(w, h)
            }
            "scale" => Step::Scale(number_arg(rest)?),
            "theme" => match rest {
                "dark" | "sombre" => Step::Theme(true),
                "light" | "clair" => Step::Theme(false),
                _ => return Err(fail("`theme dark` ou `theme light`")),
            },
            "tree-expand" if !rest.is_empty() => Step::TreeExpand(rest.to_owned()),
            "log" => Step::Log(rest.to_owned()),
            "external-write" => match rest.split_once(char::is_whitespace) {
                Some((path, text)) => Step::ExternalWrite {
                    path: PathBuf::from(path),
                    text: text.trim_start().replace("\\n", "\n"),
                },
                None => return Err(fail("`external-write <chemin> <texte>`")),
            },
            "ui" if !rest.is_empty() => {
                let (command, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                Step::Ui {
                    command: command.to_owned(),
                    arg: arg.trim().to_owned(),
                }
            }
            "quit" => Step::Quit,
            other => return Err(fail(&format!("étape inconnue ou incomplète : « {other} »"))),
        };
        steps.push((number, step));
    }
    Ok(steps)
}

/// Événements à injecter dans une image.
#[derive(Default)]
pub struct Injection {
    pub events: Vec<Event>,
}

impl Injection {
    pub fn click(pos: Pos2) -> Vec<Injection> {
        Self::click_with(pos, PointerButton::Primary)
    }

    pub fn click_with(pos: Pos2, which: PointerButton) -> Vec<Injection> {
        let button = |pressed| Event::PointerButton {
            pos,
            button: which,
            pressed,
            modifiers: Modifiers::NONE,
        };
        // Deux déplacements : egui ne sait ce qui est survolé qu'à l'image suivante, et le
        // premier clic d'une session tombait à côté sans ce délai.
        vec![
            Injection {
                events: vec![Event::PointerMoved(pos)],
            },
            Injection {
                events: vec![Event::PointerMoved(pos)],
            },
            Injection {
                events: vec![button(true)],
            },
            Injection {
                events: vec![button(false)],
            },
        ]
    }
}

/// L'exécution en cours d'un scénario.
pub struct Scenario {
    pub name: String,
    steps: VecDeque<(usize, Step)>,
    /// Images à laisser passer avant l'étape suivante.
    pub settle: u32,
    pub wait_until: Option<Instant>,
    pub screenshot: Option<Arc<AtomicBool>>,
    pub waiting_idle: bool,
    pub idle_frames: u32,
    pub idle_since: Instant,
    pub finished: bool,
}

impl Scenario {
    pub fn new(name: String, steps: Vec<(usize, Step)>) -> Self {
        Self {
            name,
            steps: steps.into(),
            settle: 3,
            wait_until: None,
            screenshot: None,
            waiting_idle: false,
            idle_frames: 0,
            idle_since: Instant::now(),
            finished: false,
        }
    }

    pub fn next(&mut self) -> Option<(usize, Step)> {
        self.steps.pop_front()
    }

    pub fn wait_ms(&mut self, ms: u64) {
        self.wait_until = Some(Instant::now() + Duration::from_millis(ms));
    }

    pub fn start_idle_wait(&mut self) {
        self.waiting_idle = true;
        self.idle_frames = 0;
        self.idle_since = Instant::now();
    }

    pub fn fail(&mut self, line: usize, message: &str) {
        eprintln!("[scénario {}:{line}] ÉCHEC : {message}", self.name);
        FAILED.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_step_kind() {
        let text = "# commentaire\n\
            project C:/projets/essai\n\
            file src/a.php:12:3\n\
            action nav.goto_file\n\
            type usct\n\
            key Ctrl+Maj+N\n\
            key Down\n\
            click 10 20.5\n\
            wait 250\n\
            wait-idle\n\
            screenshot out/a.png\n\
            state out/a.json\n\
            resize 1280 720\n\
            scale 1.5\n\
            theme light\n\
            tree-expand src\n\
            log bonjour\n\
            external-write C:/t/a.txt un\\ndeux\n\
            ui merge.accept 2 left\n\
            quit\n";
        let steps = parse(text).unwrap();
        assert_eq!(steps.len(), 19);
        assert_eq!(
            steps[17].1,
            Step::Ui {
                command: "merge.accept".into(),
                arg: "2 left".into()
            }
        );
        assert_eq!(
            steps[0].1,
            Step::Project {
                path: PathBuf::from("C:/projets/essai"),
                wait: true
            }
        );
        assert_eq!(
            steps[1].1,
            Step::File {
                path: PathBuf::from("src/a.php"),
                line: Some(12),
                column: Some(3)
            }
        );
        assert_eq!(steps[3].1, Step::Type("usct".into()));
        match &steps[4].1 {
            Step::Key(c) => {
                assert!(c.modifiers.ctrl && c.modifiers.shift && !c.modifiers.alt);
                assert_eq!(c.key, Key::N);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(steps[5].0, 7);
    }

    #[test]
    fn drive_letters_are_not_line_numbers() {
        let steps = parse("file C:/dossier/fichier.js\nfile C:/d/f.js:7").unwrap();
        assert_eq!(
            steps[0].1,
            Step::File {
                path: PathBuf::from("C:/dossier/fichier.js"),
                line: None,
                column: None
            }
        );
        assert_eq!(
            steps[1].1,
            Step::File {
                path: PathBuf::from("C:/d/f.js"),
                line: Some(7),
                column: None
            }
        );
    }

    #[test]
    fn errors_name_the_line() {
        let error = parse("quit\nfrobnicate 1").unwrap_err();
        assert!(error.starts_with("ligne 2"), "{error}");
        assert!(parse("key Ctrl+").is_err());
        assert!(parse("theme rouge").is_err());
    }

    #[test]
    fn combos_accept_french_names() {
        let c = parse_combo("Alt+Gauche").unwrap();
        assert_eq!(c.key, Key::ArrowLeft);
        assert!(c.modifiers.alt);
        assert_eq!(parse_combo("Alt+F12").unwrap().key, Key::F12);
        assert_eq!(parse_combo("Ctrl+Tab").unwrap().key, Key::Tab);
    }
}
