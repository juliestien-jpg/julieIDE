//! Chargement, enregistrement et surveillance par tâches de fond : rien de ce
//! qui touche le disque ne passe par le fil de l'interface.

use std::path::PathBuf;

use julie_core::{Domain, TaskHandle, Tasks};

use crate::document::{
    check_disk, load_file, DiskStamp, DiskState, LoadedFile, SaveOutcome, SaveRequest,
};
use crate::error::TextError;

/// Événements publiés sous `Domain::Text` (`event.take::<TextEvent>()` pour
/// prendre le texte sans copie).
#[derive(Debug)]
pub enum TextEvent {
    /// Fichier lu (`text.load`).
    Loaded(LoadedFile),
    /// Lecture impossible (`text.load`) : binaire, trop gros, introuvable…
    LoadFailed { path: PathBuf, error: TextError },
    /// Fichier écrit (`text.save`) : à rendre au document par `mark_saved`.
    Saved(SaveOutcome),
    /// Écriture refusée (`text.save`) : lecture seule, caractère hors de
    /// l'encodage (`TextError::Core(Error::Unencodable { .. })`)… Le document
    /// reste modifié.
    SaveFailed { path: PathBuf, error: TextError },
    /// État sur le disque des fichiers demandés (`text.check_disk`).
    DiskChecked(Vec<(PathBuf, DiskState)>),
}

/// Lit un fichier ; publie `Loaded` ou `LoadFailed`.
pub fn spawn_load(tasks: &Tasks, path: PathBuf) -> TaskHandle {
    tasks.spawn(Domain::Text, "text.load", move |ctx| {
        match load_file(&path) {
            Ok(loaded) => ctx.publish(TextEvent::Loaded(loaded)),
            Err(error) => {
                let message = error.to_string();
                ctx.publish(TextEvent::LoadFailed { path, error });
                return Err(message.into());
            }
        }
        Ok(())
    })
}

/// Écrit un document (préparé par `Document::save_request`) ; publie `Saved` ou
/// `SaveFailed`.
pub fn spawn_save(tasks: &Tasks, request: SaveRequest) -> TaskHandle {
    tasks.spawn(Domain::Text, "text.save", move |ctx| {
        let path = request.path.clone();
        match request.execute() {
            Ok(outcome) => ctx.publish(TextEvent::Saved(outcome)),
            Err(error) => {
                let message = error.to_string();
                ctx.publish(TextEvent::SaveFailed { path, error });
                return Err(message.into());
            }
        }
        Ok(())
    })
}

/// Compare l'état des fichiers sur le disque à celui qu'on en connaît ; publie
/// `DiskChecked` avec tous les fichiers, changés ou non.
pub fn spawn_check_disk(tasks: &Tasks, files: Vec<(PathBuf, Option<DiskStamp>)>) -> TaskHandle {
    tasks.spawn(Domain::Text, "text.check_disk", move |ctx| {
        let mut states = Vec::with_capacity(files.len());
        for (path, stamp) in files {
            ctx.check_cancelled()?;
            let state = check_disk(&path, stamp);
            states.push((path, state));
        }
        ctx.publish(TextEvent::DiskChecked(states));
        Ok(())
    })
}
