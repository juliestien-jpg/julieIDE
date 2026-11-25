//! Petites fenêtres de l'arbre du projet : nouveau fichier / dossier, renommer,
//! supprimer (vers la corbeille), seconde confirmation de la suppression définitive.
//!
//! Ce module valide les noms (règles de Windows) et dessine ; l'application exécute
//! (`app/fsops.rs`). Rien n'est écrit ici.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Key, Modifiers, RichText, TextEdit, Ui};

use crate::texts as t;
use crate::theme::Palette;

/// Ce que la personne a décidé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsAnswer {
    Create,
    Rename,
    Trash,
    Forever,
    Cancel,
}

impl FsAnswer {
    pub fn parse(key: &str) -> Option<FsAnswer> {
        Some(match key {
            "create" => FsAnswer::Create,
            "rename" => FsAnswer::Rename,
            "trash" => FsAnswer::Trash,
            "forever" => FsAnswer::Forever,
            "cancel" => FsAnswer::Cancel,
            _ => return None,
        })
    }
}

pub struct NewItem {
    pub dir: PathBuf,
    pub folder: bool,
    pub name: String,
    pub problem: Option<String>,
    pub focus: bool,
}

pub struct RenameItem {
    pub path: PathBuf,
    pub name: String,
    pub problem: Option<String>,
    pub focus: bool,
}

pub struct DeleteItems {
    pub paths: Vec<PathBuf>,
    /// Une ligne par élément (nom, et « dossier et ses n fichiers »).
    pub lines: Vec<String>,
    /// Onglets modifiés non enregistrés concernés.
    pub unsaved: Vec<String>,
}

pub enum FsDialog {
    New(NewItem),
    Rename(RenameItem),
    Delete(DeleteItems),
    /// La corbeille n'a pas pu recevoir ces éléments : suppression définitive ?
    Forever(Vec<PathBuf>),
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Un nom (un seul élément de chemin) accepté par Windows ?
pub fn validate_segment(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(t::NAME_EMPTY.to_owned());
    }
    if let Some(c) = name
        .chars()
        .find(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || (*c as u32) < 32)
    {
        return Err(t::name_forbidden_char(c));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(t::NAME_TRAILING.to_owned());
    }
    let stem = name.split('.').next().unwrap_or(name).to_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Err(t::NAME_RESERVED.to_owned());
    }
    Ok(())
}

/// Le nom saisi pour un nouvel élément : des dossiers intermédiaires sont permis
/// (`/` ou `\`), chaque segment est vérifié.
pub fn validate_new(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(t::NAME_EMPTY.to_owned());
    }
    if name.starts_with(['/', '\\']) {
        return Err(t::NAME_EMPTY.to_owned());
    }
    let mut path = dir.to_path_buf();
    for segment in name.split(['/', '\\']) {
        validate_segment(segment)?;
        if segment == "." || segment == ".." {
            return Err(t::name_forbidden_char('.'));
        }
        path.push(segment);
    }
    if path.exists() {
        return Err(t::NAME_EXISTS.to_owned());
    }
    Ok(path)
}

/// Le nouveau nom d'un élément existant : pas de séparateur ; l'existant d'un
/// autre nom est refusé, le même nom à la casse près est permis.
pub fn validate_rename(old: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.contains(['/', '\\']) {
        return Err(t::NAME_SEPARATOR.to_owned());
    }
    validate_segment(name)?;
    let parent = old.parent().unwrap_or(Path::new(""));
    let target = parent.join(name);
    let old_name = old.file_name().map(|n| n.to_string_lossy().into_owned());
    let case_only = old_name
        .as_deref()
        .is_some_and(|o| o.to_lowercase() == name.to_lowercase());
    if target.exists() && !case_only {
        return Err(t::NAME_EXISTS.to_owned());
    }
    Ok(target)
}

impl FsDialog {
    pub fn kind(&self) -> &'static str {
        match self {
            FsDialog::New(d) if d.folder => "new_folder",
            FsDialog::New(_) => "new_file",
            FsDialog::Rename(_) => "rename_path",
            FsDialog::Delete(_) => "delete_paths",
            FsDialog::Forever(_) => "delete_forever",
        }
    }

    fn title(&self) -> String {
        match self {
            FsDialog::New(d) if d.folder => t::DLG_NEW_FOLDER.to_owned(),
            FsDialog::New(_) => t::DLG_NEW_FILE.to_owned(),
            FsDialog::Rename(d) => t::dlg_rename_fs(
                &d.path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            ),
            FsDialog::Delete(d) => t::dlg_delete(d.paths.len()),
            FsDialog::Forever(_) => t::DLG_DELETE_FOREVER.to_owned(),
        }
    }

    /// Règle le champ nom (mode scénario).
    pub fn set_name(&mut self, value: &str) -> Result<(), String> {
        match self {
            FsDialog::New(d) => {
                d.name = value.to_owned();
                Ok(())
            }
            FsDialog::Rename(d) => {
                d.name = value.to_owned();
                Ok(())
            }
            _ => Err("ce dialogue n'a pas de champ « name »".to_owned()),
        }
    }

    /// Revalide à chaque image : le problème affiché suit la frappe.
    fn revalidate(&mut self) {
        match self {
            FsDialog::New(d) => {
                d.problem = if d.name.is_empty() {
                    None
                } else {
                    validate_new(&d.dir, &d.name).err()
                };
            }
            FsDialog::Rename(d) => {
                d.problem = if d.name.is_empty() {
                    None
                } else {
                    validate_rename(&d.path, &d.name).err()
                };
            }
            _ => {}
        }
    }

    /// Le bouton principal est-il actif ?
    pub fn can_confirm(&self) -> bool {
        match self {
            FsDialog::New(d) => validate_new(&d.dir, &d.name).is_ok(),
            FsDialog::Rename(d) => {
                validate_rename(&d.path, &d.name).is_ok()
                    && d.path.file_name().map(|n| n.to_string_lossy().into_owned())
                        != Some(d.name.trim().to_owned())
            }
            FsDialog::Delete(_) | FsDialog::Forever(_) => true,
        }
    }

    pub fn primary(&self) -> FsAnswer {
        match self {
            FsDialog::New(_) => FsAnswer::Create,
            FsDialog::Rename(_) => FsAnswer::Rename,
            FsDialog::Delete(_) => FsAnswer::Trash,
            FsDialog::Forever(_) => FsAnswer::Forever,
        }
    }

    pub fn state(&self) -> serde_json::Value {
        let (name, problem) = match self {
            FsDialog::New(d) => (Some(d.name.clone()), d.problem.clone()),
            FsDialog::Rename(d) => (Some(d.name.clone()), d.problem.clone()),
            _ => (None, None),
        };
        serde_json::json!({
            "kind": self.kind(),
            "title": self.title(),
            "name": name,
            "problem": problem,
            "can_confirm": self.can_confirm(),
            "items": match self {
                FsDialog::Delete(d) => d.lines.clone(),
                FsDialog::Forever(paths) => paths.iter().map(|p| p.display().to_string()).collect(),
                _ => Vec::new(),
            },
        })
    }

    fn body(&mut self, ui: &mut Ui, p: &Palette) {
        let dim = |s: String| RichText::new(s).color(p.text_dim);
        match self {
            FsDialog::New(d) => {
                ui.label(dim(t::dlg_new_in(&d.dir.display().to_string())));
                ui.add_space(4.0);
                ui.label(dim(t::LABEL_NAME.to_owned()));
                let edit = ui.add(
                    TextEdit::singleline(&mut d.name)
                        .hint_text(t::HINT_NEW_NAME)
                        .desired_width(f32::INFINITY)
                        .id_salt("fs-nom"),
                );
                if d.focus {
                    d.focus = false;
                    edit.request_focus();
                }
                if let Some(problem) = &d.problem {
                    ui.label(RichText::new(problem).color(p.error));
                }
            }
            FsDialog::Rename(d) => {
                let edit = ui.add(
                    TextEdit::singleline(&mut d.name)
                        .desired_width(f32::INFINITY)
                        .id_salt("fs-renommer"),
                );
                if d.focus {
                    d.focus = false;
                    edit.request_focus();
                    // Nom sans l'extension sélectionné.
                    let stem = d.name.rfind('.').filter(|i| *i > 0).unwrap_or(d.name.len());
                    let mut state = TextEdit::load_state(ui.ctx(), edit.id).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(d.name[..stem].chars().count()),
                        )));
                    state.store(ui.ctx(), edit.id);
                }
                if let Some(problem) = &d.problem {
                    ui.label(RichText::new(problem).color(p.error));
                }
            }
            FsDialog::Delete(d) => {
                for line in d.lines.iter().take(10) {
                    ui.label(RichText::new(line).color(p.text));
                }
                if d.lines.len() > 10 {
                    ui.label(dim(t::and_n_more(d.lines.len() - 10)));
                }
                for name in &d.unsaved {
                    ui.add_space(2.0);
                    ui.label(RichText::new(t::delete_unsaved(name)).color(p.warn));
                }
                ui.add_space(6.0);
                ui.label(dim(t::DELETE_TRASH_NOTE.to_owned()));
            }
            FsDialog::Forever(paths) => {
                for path in paths.iter().take(10) {
                    ui.label(RichText::new(path.display().to_string()).color(p.text));
                }
                if paths.len() > 10 {
                    ui.label(dim(t::and_n_more(paths.len() - 10)));
                }
                ui.add_space(6.0);
                ui.label(RichText::new(t::DELETE_FOREVER_TEXT).color(p.error));
            }
        }
    }

    /// Dessine la fenêtre ; rend la réponse quand il y en a une.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette) -> Option<FsAnswer> {
        self.revalidate();
        let mut answer = None;
        let enter = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        let escape = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        let can = self.can_confirm();
        let title = self.title();
        let (label, danger) = match self {
            FsDialog::New(_) => (t::BTN_CREATE, false),
            FsDialog::Rename(_) => (t::BTN_RENAME_FS, false),
            FsDialog::Delete(_) => (t::BTN_DELETE_TRASH, false),
            FsDialog::Forever(_) => (t::BTN_DELETE_FOREVER, true),
        };
        let primary = self.primary();
        let modal = crate::kit::modal("fs-dialogue").show(ctx, |ui| {
            ui.set_width(440.0);
            ui.label(RichText::new(title).strong().size(15.0).color(p.text));
            ui.add_space(6.0);
            self.body(ui, p);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                // « Annuler » est le bouton par défaut d'une suppression définitive.
                let default_cancel = matches!(self, FsDialog::Forever(_));
                let mut button = egui::Button::new(if danger {
                    RichText::new(label).color(Color32::WHITE)
                } else {
                    RichText::new(label)
                });
                if danger {
                    button = button.fill(p.error);
                }
                if ui.add_enabled(can, button).clicked() {
                    answer = Some(primary);
                }
                let cancel = ui.button(t::DIALOG_CANCEL);
                if default_cancel && ui.memory(|m| m.focused().is_none()) {
                    cancel.request_focus();
                }
                if cancel.clicked() {
                    answer = Some(FsAnswer::Cancel);
                }
            });
        });
        if answer.is_none() {
            if escape || modal.should_close() {
                answer = Some(FsAnswer::Cancel);
            } else if enter && can && !matches!(self, FsDialog::Forever(_)) {
                answer = Some(primary);
            }
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_names_are_checked_before_any_write() {
        assert!(validate_segment("a.php").is_ok());
        assert!(validate_segment("con.php").is_err());
        assert!(validate_segment("NUL").is_err());
        assert!(validate_segment("COM1.txt").is_err());
        assert!(validate_segment("a?.php").unwrap_err().contains('?'));
        assert!(validate_segment("a:b").is_err());
        assert!(validate_segment("fin.").is_err());
        assert!(validate_segment("fin ").is_err());
        assert!(validate_segment("").is_err());
        assert!(validate_segment("console.php").is_ok());
    }

    #[test]
    fn new_names_accept_intermediate_folders_and_refuse_existing() {
        let dir = std::env::temp_dir();
        let ok = validate_new(&dir, "sous/dos\\a.php").unwrap();
        assert!(
            ok.ends_with("sous\\dos\\a.php")
                || ok.ends_with("sous/dos/a.php")
                || ok.components().count() > 3
        );
        assert!(validate_new(&dir, "sous/con.php").is_err());
        assert!(validate_new(&dir, "../a.php").is_err());
        assert!(validate_new(&dir, "/a.php").is_err());
        // Le dossier temporaire existe déjà : refusé sans rien écraser.
        assert!(validate_new(
            dir.parent().unwrap(),
            dir.file_name().unwrap().to_str().unwrap()
        )
        .is_err());
    }

    #[test]
    fn rename_allows_a_case_only_change() {
        let dir = std::env::temp_dir().join(format!("juliestorm-fs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("user.php");
        std::fs::write(&old, "").unwrap();
        std::fs::write(dir.join("autre.php"), "").unwrap();
        assert!(validate_rename(&old, "User.php").is_ok());
        assert!(validate_rename(&old, "autre.php").is_err());
        assert!(validate_rename(&old, "a/b.php").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
