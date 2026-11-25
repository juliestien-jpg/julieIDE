//! Opérations sur les fichiers de l'arbre : nouveau fichier / dossier, renommer
//! (`git mv` si le fichier est suivi), supprimer vers la corbeille (`julie_core::trash`),
//! suppression définitive après seconde confirmation. Chaque opération qui touche
//! le disque tourne dans un fil ; le résultat revient par un `Msg`.

use super::*;

use julie_core::Error as CoreError;

use crate::fsdialog::{self, DeleteItems, FsAnswer, FsDialog, NewItem, RenameItem};
use crate::project::TrashResult;

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl JulieStormApp {
    // ----- Ouverture des fenêtres --------------------------------------------------

    pub(super) fn open_new_item(&mut self, dir: PathBuf, folder: bool) {
        self.close_floating();
        self.fs_dialog = Some(FsDialog::New(NewItem {
            dir,
            folder,
            name: String::new(),
            problem: None,
            focus: true,
        }));
    }

    pub(super) fn open_rename_item(&mut self, path: PathBuf) {
        self.close_floating();
        self.fs_dialog = Some(FsDialog::Rename(RenameItem {
            name: display_name(&path),
            path,
            problem: None,
            focus: true,
        }));
    }

    pub(super) fn open_delete_items(&mut self, paths: Vec<PathBuf>) {
        self.close_floating();
        let root = self.project_root();
        let mut lines = Vec::new();
        for path in &paths {
            let name = display_name(path);
            if path.is_dir() {
                match self.count_files_under(path, root.as_deref()) {
                    Some(n) => lines.push(t::delete_folder_files(&name, n)),
                    None => lines.push(name),
                }
            } else {
                lines.push(name);
            }
        }
        let unsaved: Vec<String> = self
            .editors
            .items
            .iter()
            .filter(|e| e.document.is_modified())
            .filter_map(|e| e.path())
            .filter(|p| paths.iter().any(|q| p.starts_with(q)))
            .map(display_name)
            .collect();
        self.fs_dialog = Some(FsDialog::Delete(DeleteItems {
            paths,
            lines,
            unsaved,
        }));
    }

    /// Nombre de fichiers (de l'index, déjà en mémoire) sous ce dossier.
    fn count_files_under(&self, dir: &Path, root: Option<&Path>) -> Option<usize> {
        let project = self.project.as_ref()?;
        if !project.index.is_ready() {
            return None;
        }
        let relative = dir
            .strip_prefix(root?)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        let prefix = format!("{relative}/");
        Some(project.index.with_index(|files| {
            files
                .entries()
                .iter()
                .filter(|e| e.relative().starts_with(prefix.as_str()))
                .count()
        }))
    }

    // ----- Dessin et réponses ------------------------------------------------------

    pub(super) fn draw_fs_dialog(&mut self, ctx: &egui::Context, p: &Palette) {
        let Some(mut dialog) = self.fs_dialog.take() else {
            return;
        };
        match dialog.show(ctx, p) {
            Some(answer) => self.fs_answer(dialog, answer),
            None => self.fs_dialog = Some(dialog),
        }
    }

    pub(super) fn fs_answer(&mut self, dialog: FsDialog, answer: FsAnswer) {
        match (dialog, answer) {
            (_, FsAnswer::Cancel) => self.tree.request_focus(),
            (FsDialog::New(d), FsAnswer::Create) => self.do_create(d),
            (FsDialog::Rename(d), FsAnswer::Rename) => self.do_rename(d),
            (FsDialog::Delete(d), FsAnswer::Trash) => self.do_trash(d.paths),
            (FsDialog::Forever(paths), FsAnswer::Forever) => self.do_forever(paths),
            (other, _) => self.fs_dialog = Some(other),
        }
        self.ctx.request_repaint();
    }

    /// `dialog.choose <clé>` / `dialog.set name …` du mode scénario, quand une fenêtre
    /// de l'arbre est ouverte.
    pub(super) fn fs_dialog_command(
        &mut self,
        command: &str,
        arg: &str,
    ) -> Option<Result<(), String>> {
        self.fs_dialog.as_ref()?;
        match command {
            "dialog.set" => {
                let (field, value) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
                if field != "name" {
                    return Some(Err(format!("champ inconnu : « {field} » (name)")));
                }
                let dialog = self.fs_dialog.as_mut()?;
                Some(dialog.set_name(value.trim_start()))
            }
            "dialog.choose" => {
                let answer = match FsAnswer::parse(arg.trim()) {
                    Some(answer) => answer,
                    None => return Some(Err(format!("bouton inconnu : « {arg} »"))),
                };
                let dialog = self.fs_dialog.take()?;
                if answer != FsAnswer::Cancel && answer != dialog.primary() {
                    self.fs_dialog = Some(dialog);
                    return Some(Err(format!("pas de bouton « {arg} » dans ce dialogue")));
                }
                if answer != FsAnswer::Cancel && !dialog.can_confirm() {
                    self.fs_dialog = Some(dialog);
                    return Some(Err(format!("bouton « {arg} » grisé")));
                }
                self.fs_answer(dialog, answer);
                Some(Ok(()))
            }
            _ => None,
        }
    }

    // ----- Création -----------------------------------------------------------------

    fn do_create(&mut self, d: NewItem) {
        let path = match fsdialog::validate_new(&d.dir, &d.name) {
            Ok(path) => path,
            Err(problem) => {
                self.fs_dialog = Some(FsDialog::New(NewItem {
                    problem: Some(problem),
                    ..d
                }));
                return;
            }
        };
        let channel = self.channel.clone();
        let folder = d.folder;
        let shown = self
            .project_root()
            .and_then(|r| path.strip_prefix(r).ok().map(|p| p.display().to_string()))
            .unwrap_or_else(|| path.display().to_string());
        std::thread::spawn(move || {
            let result: Result<(), String> = if folder {
                std::fs::create_dir_all(&path).map_err(|e| e.to_string())
            } else {
                path.parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        julie_core::file::write_atomic(&path, b"").map_err(|e| e.to_string())
                    })
            };
            match result {
                Ok(()) => {
                    channel.send(Msg::FilesChanged {
                        text: Some(t::fs_created(&shown)),
                        error: false,
                        refresh: true,
                        reveal: Some(path.clone()),
                    });
                    if !folder {
                        channel.send(Msg::OpenPath { path, line: None });
                    }
                }
                Err(error) => channel.send(Msg::Notice {
                    text: t::fs_failed(&error),
                    error: true,
                }),
            }
        });
    }

    // ----- Renommage ----------------------------------------------------------------

    fn do_rename(&mut self, d: RenameItem) {
        let target = match fsdialog::validate_rename(&d.path, &d.name) {
            Ok(target) => target,
            Err(problem) => {
                self.fs_dialog = Some(FsDialog::Rename(RenameItem {
                    problem: Some(problem),
                    ..d
                }));
                return;
            }
        };
        if target == d.path {
            return;
        }
        // Un fichier suivi par Git : `git mv` (Git voit un renommage). Sinon (`NotTracked`
        // ou pas de dépôt), renommage simple.
        match self.vcs_pair() {
            Some((vcs, repo)) => {
                self.pending_rename = Some((d.path.clone(), target.clone()));
                vcs.move_path(&repo, d.path, target);
            }
            None => self.plain_rename(d.path, target),
        }
    }

    pub(super) fn plain_rename(&mut self, from: PathBuf, to: PathBuf) {
        let channel = self.channel.clone();
        std::thread::spawn(move || match std::fs::rename(&from, &to) {
            Ok(()) => channel.send(Msg::Moved { from, to }),
            Err(error) => channel.send(Msg::Notice {
                text: t::fs_failed(&error.to_string()),
                error: true,
            }),
        });
    }

    /// Un élément a changé de chemin (`git mv` ou renommage simple) : les onglets suivent.
    pub(super) fn after_move(&mut self, from: &Path, to: &Path) {
        self.pending_rename = None;
        self.editors.rename_path(from, to);
        self.tree.refresh();
        self.tree.expand_path(to);
        self.refresh_git();
        self.toast(
            t::fs_renamed(&display_name(from), &display_name(to)),
            false,
            None,
        );
    }

    // ----- Suppression --------------------------------------------------------------

    fn do_trash(&mut self, paths: Vec<PathBuf>) {
        let channel = self.channel.clone();
        std::thread::spawn(move || {
            let result = match julie_core::trash::move_to_trash(&paths) {
                Ok(()) => TrashResult::Done,
                Err(CoreError::TrashUnavailable { remaining }) => {
                    TrashResult::Unavailable(remaining)
                }
                Err(CoreError::TrashFailed { remaining, code }) => TrashResult::Failed {
                    text: format!("0x{code:08X}"),
                    remaining,
                },
                Err(other) => TrashResult::Failed {
                    text: other.to_string(),
                    remaining: Vec::new(),
                },
            };
            channel.send(Msg::Trashed { paths, result });
        });
    }

    pub(super) fn on_trashed(&mut self, paths: Vec<PathBuf>, result: TrashResult) {
        match result {
            TrashResult::Done => {
                self.after_deleted(&paths);
                self.toast(t::fs_trashed(paths.len()), false, None);
            }
            TrashResult::Unavailable(remaining) => {
                let gone: Vec<PathBuf> = paths
                    .iter()
                    .filter(|p| !remaining.contains(p))
                    .cloned()
                    .collect();
                self.after_deleted(&gone);
                // Rien n'a été supprimé définitivement : seconde confirmation, « Annuler »
                // par défaut.
                self.fs_dialog = Some(FsDialog::Forever(remaining));
            }
            TrashResult::Failed { text, remaining } => {
                let gone: Vec<PathBuf> = paths
                    .iter()
                    .filter(|p| !remaining.contains(p) && !p.exists())
                    .cloned()
                    .collect();
                self.after_deleted(&gone);
                self.toast(t::fs_failed(&text), true, None);
            }
        }
    }

    fn do_forever(&mut self, paths: Vec<PathBuf>) {
        let channel = self.channel.clone();
        std::thread::spawn(move || {
            let mut done = 0usize;
            let mut failure = None;
            for path in &paths {
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
                match result {
                    Ok(()) => done += 1,
                    Err(error) => {
                        failure = Some(error.to_string());
                        break;
                    }
                }
            }
            channel.send(Msg::FilesChanged {
                text: Some(match &failure {
                    Some(error) => t::fs_failed(error),
                    None => t::fs_deleted_forever(done),
                }),
                error: failure.is_some(),
                refresh: true,
                reveal: None,
            });
        });
    }

    /// Les éléments ne sont plus sur le disque : onglets non modifiés fermés, onglets
    /// modifiés gardés (bandeau « supprimé du disque »), arbre et état Git relus.
    fn after_deleted(&mut self, paths: &[PathBuf]) {
        let gone: Vec<usize> = self
            .editors
            .items
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.path()
                    .is_some_and(|p| paths.iter().any(|q| p.starts_with(q)))
                    && !e.document.is_modified()
            })
            .map(|(i, _)| i)
            .collect();
        for index in gone.into_iter().rev() {
            self.editors.close(index);
        }
        self.editors.check_disk(&self.tasks);
        self.tree.refresh();
        self.refresh_git();
    }

    /// Envoie ces fichiers (les « ajoutés » d'une annulation) à la corbeille, sans
    /// question : la case a été cochée dans le dialogue d'annulation.
    pub(super) fn trash_added(&mut self, paths: Vec<PathBuf>) {
        let existing: Vec<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
        if existing.is_empty() {
            return;
        }
        let channel = self.channel.clone();
        std::thread::spawn(move || {
            let result = julie_core::trash::move_to_trash(&existing);
            let (text, error) = match result {
                Ok(()) => (t::trashed_added(existing.len()), false),
                Err(error) => (t::fs_failed(&error.to_string()), true),
            };
            channel.send(Msg::FilesChanged {
                text: Some(text),
                error,
                refresh: true,
                reveal: None,
            });
        });
    }
}
