//! Ce qui circule sur le bus.
//!
//! Le noyau ne connaît pas les types des crates métier : chacune définit son propre
//! type d'événement (par exemple `julie_vcs::VcsEvent`) et le publie dans un
//! [`DomainEvent`], qui le transporte sans le connaître. `app`, qui connaît toutes
//! les crates, le retrouve avec [`DomainEvent::get`].

use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::task::{TaskEvent, TaskId};

/// Ce que les fils de fond publient pour l'interface.
#[derive(Debug, Clone)]
pub enum Event {
    /// Un projet vient d'être ouvert à cette racine.
    ProjectOpened(PathBuf),
    /// Des fichiers ont changé sur le disque, hors de l'IDE (chemins absolus).
    FilesChangedOnDisk(Vec<PathBuf>),
    /// Cycle de vie d'une tâche de fond : début, progression, fin.
    Task(TaskEvent),
    /// Résultat publié par une crate métier.
    Domain(DomainEvent),
}

/// Les domaines métier : un par crate. Sert à aiguiller les événements vers la
/// bonne vue et à regrouper les tâches dans l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Domain {
    Text,
    Diff,
    Vcs,
    Index,
    PhpUnit,
    Terminal,
}

/// Charge utile d'une crate métier, transportée sans que le noyau en connaisse le type.
#[derive(Clone)]
pub struct DomainEvent {
    pub domain: Domain,
    /// La tâche qui a publié, quand la publication passe par
    /// [`crate::task::TaskContext::publish`]. Permet d'écarter les résultats d'une
    /// demande périmée (recherche relancée, par exemple).
    pub task: Option<TaskId>,
    payload: Arc<dyn Any + Send + Sync>,
    type_name: &'static str,
}

impl DomainEvent {
    pub fn new<T: Any + Send + Sync>(domain: Domain, payload: T) -> Self {
        Self {
            domain,
            task: None,
            payload: Arc::new(payload),
            type_name: std::any::type_name::<T>(),
        }
    }

    pub fn from_task<T: Any + Send + Sync>(domain: Domain, task: TaskId, payload: T) -> Self {
        Self {
            task: Some(task),
            ..Self::new(domain, payload)
        }
    }

    /// La charge utile si elle est du type `T`, sinon `None`.
    pub fn get<T: Any>(&self) -> Option<&T> {
        self.payload.downcast_ref::<T>()
    }

    /// Prend possession de la charge utile sans la copier (gros résultats). Rend
    /// l'événement intact si le type ne correspond pas ou si un clone existe encore.
    pub fn take<T: Any + Send + Sync>(self) -> Result<T, Self> {
        let Self {
            domain,
            task,
            payload,
            type_name,
        } = self;
        let rebuild = |payload| Self {
            domain,
            task,
            payload,
            type_name,
        };
        match payload.downcast::<T>() {
            Ok(typed) => Arc::try_unwrap(typed).map_err(|typed| rebuild(typed)),
            Err(payload) => Err(rebuild(payload)),
        }
    }

    /// Nom du type transporté, pour les journaux.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }
}

impl fmt::Debug for DomainEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DomainEvent")
            .field("domain", &self.domain)
            .field("task", &self.task)
            .field("payload", &self.type_name)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum FakeVcsEvent {
        StatusChanged(Vec<String>),
    }

    #[test]
    fn payload_is_found_by_type_and_only_by_its_type() {
        let event = DomainEvent::new(Domain::Vcs, FakeVcsEvent::StatusChanged(vec!["a".into()]));
        assert_eq!(
            event.get::<FakeVcsEvent>(),
            Some(&FakeVcsEvent::StatusChanged(vec!["a".into()]))
        );
        assert!(event.get::<String>().is_none());
        assert!(event.type_name().ends_with("FakeVcsEvent"));
    }

    #[test]
    fn take_gives_ownership_or_gives_the_event_back() {
        let event = DomainEvent::new(Domain::Index, vec![1u32, 2, 3]);
        let event = event.take::<String>().expect_err("mauvais type");
        assert_eq!(event.domain, Domain::Index);

        let shared = event.clone();
        let event = event
            .take::<Vec<u32>>()
            .expect_err("un clone existe encore");
        drop(shared);
        assert_eq!(event.take::<Vec<u32>>().unwrap(), vec![1, 2, 3]);
    }
}
