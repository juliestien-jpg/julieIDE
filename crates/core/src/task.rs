//! Exécuteur de tâches de fond : un fil par tâche, annulation coopérative,
//! progression publiée sur le bus.
//!
//! Tout ce qui lit le disque, lance un processus ou calcule plus de quelques
//! millisecondes passe par [`Tasks::spawn`]. La tâche rend ses résultats en
//! publiant des événements ([`TaskContext::publish`]) ; l'interface suit son cycle
//! de vie par les [`TaskEvent`].

use std::any::Any;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::bus::Publisher;
use crate::error::{Error, Result};
use crate::event::{Domain, DomainEvent, Event};

/// Intervalle minimal entre deux événements de progression d'une même tâche.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// Ce que rend le travail d'une tâche. L'erreur est en boîte pour que `?`
/// convertisse d'office l'erreur de n'importe quelle crate métier.
pub type TaskResult = std::result::Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Identifiant d'une tâche, unique pour la durée de vie de l'exécuteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u64);

/// Jeton d'annulation coopérative : la tâche le consulte régulièrement.
/// Clonable ; tous les clones partagent le même état.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// `Err(Error::Cancelled)` si l'annulation est demandée : à semer dans les
    /// boucles avec `?`.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Événement du cycle de vie d'une tâche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEvent {
    pub id: TaskId,
    pub domain: Domain,
    /// Clé stable décrivant le travail (« index.scan », « vcs.status »…) :
    /// l'interface en tire le libellé dans le catalogue de textes.
    pub kind: &'static str,
    pub state: TaskState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Started,
    /// `total == 0` : durée indéterminée.
    Progress {
        done: u64,
        total: u64,
    },
    Finished(TaskOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskOutcome {
    Done,
    Cancelled,
    /// Message technique (le `Display` de l'erreur), pour le journal.
    Failed(String),
}

/// L'exécuteur. Clonable : tous les clones partagent le même registre de tâches.
#[derive(Clone)]
pub struct Tasks {
    inner: Arc<Inner>,
}

struct Inner {
    publisher: Publisher,
    next_id: AtomicU64,
    running: Mutex<HashMap<TaskId, CancelToken>>,
}

impl Tasks {
    pub fn new(publisher: Publisher) -> Self {
        Self {
            inner: Arc::new(Inner {
                publisher,
                next_id: AtomicU64::new(1),
                running: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn publisher(&self) -> &Publisher {
        &self.inner.publisher
    }

    /// Lance `work` dans un nouveau fil et rend aussitôt la main.
    ///
    /// Publie `Started` avant de rendre la main, puis `Finished` à la fin :
    /// `Done` si `work` rend `Ok`, `Cancelled` s'il rend une erreur alors que
    /// l'annulation était demandée, `Failed` sinon (panique comprise).
    /// Lâcher le [`TaskHandle`] n'arrête pas la tâche.
    pub fn spawn<F>(&self, domain: Domain, kind: &'static str, work: F) -> TaskHandle
    where
        F: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        let id = TaskId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let cancel = CancelToken::new();
        lock(&self.inner.running).insert(id, cancel.clone());

        let context = TaskContext {
            id,
            domain,
            kind,
            cancel: cancel.clone(),
            publisher: self.inner.publisher.clone(),
            last_progress: Mutex::new(None),
        };
        context.send(TaskState::Started);

        let inner = Arc::clone(&self.inner);
        let run = move || {
            let outcome = match catch_unwind(AssertUnwindSafe(|| work(&context))) {
                Ok(Ok(())) => TaskOutcome::Done,
                Ok(Err(_)) if context.cancel.is_cancelled() => TaskOutcome::Cancelled,
                Ok(Err(error)) => TaskOutcome::Failed(error.to_string()),
                Err(panic) => TaskOutcome::Failed(format!("panique : {}", panic_text(&panic))),
            };
            lock(&inner.running).remove(&id);
            context.send(TaskState::Finished(outcome));
        };
        let thread = std::thread::Builder::new()
            .name(format!("julie-{kind}"))
            .spawn(run)
            .expect("création d'un fil de tâche de fond");

        TaskHandle { id, cancel, thread }
    }

    /// Demande l'annulation d'une tâche ; sans effet si elle est déjà finie.
    pub fn cancel(&self, id: TaskId) {
        if let Some(token) = lock(&self.inner.running).get(&id) {
            token.cancel();
        }
    }

    /// Demande l'annulation de toutes les tâches en cours (fermeture du projet
    /// ou de l'application).
    pub fn cancel_all(&self) {
        for token in lock(&self.inner.running).values() {
            token.cancel();
        }
    }

    pub fn running_count(&self) -> usize {
        lock(&self.inner.running).len()
    }
}

/// Ce que reçoit le travail d'une tâche. Partageable entre les fils que la tâche
/// lancerait elle-même.
pub struct TaskContext {
    id: TaskId,
    domain: Domain,
    kind: &'static str,
    cancel: CancelToken,
    publisher: Publisher,
    last_progress: Mutex<Option<Instant>>,
}

impl TaskContext {
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Le jeton à passer aux appels annulables ([`crate::process::Command::run`]…).
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Raccourci de `cancel_token().check()`.
    pub fn check_cancelled(&self) -> Result<()> {
        self.cancel.check()
    }

    /// Signale la progression. Peut être appelé à chaque élément traité : les
    /// événements sont espacés d'au moins 50 ms, sauf le dernier (`done == total`),
    /// toujours publié.
    pub fn progress(&self, done: u64, total: u64) {
        let now = Instant::now();
        {
            let mut last = lock(&self.last_progress);
            let is_final = total > 0 && done >= total;
            let due = last.is_none_or(|at| now.duration_since(at) >= PROGRESS_INTERVAL);
            if !is_final && !due {
                return;
            }
            *last = Some(now);
        }
        self.send(TaskState::Progress { done, total });
    }

    /// Publie un résultat de la crate métier, étiqueté avec le domaine et
    /// l'identifiant de cette tâche.
    pub fn publish<T: Any + Send + Sync>(&self, payload: T) {
        self.publisher.publish(Event::Domain(DomainEvent::from_task(
            self.domain,
            self.id,
            payload,
        )));
    }

    pub fn publisher(&self) -> &Publisher {
        &self.publisher
    }

    fn send(&self, state: TaskState) {
        self.publisher.publish(Event::Task(TaskEvent {
            id: self.id,
            domain: self.domain,
            kind: self.kind,
            state,
        }));
    }
}

/// Poignée rendue par [`Tasks::spawn`].
pub struct TaskHandle {
    id: TaskId,
    cancel: CancelToken,
    thread: JoinHandle<()>,
}

impl TaskHandle {
    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Attend la fin de la tâche. **Bloquant** : pour les tests et pour une tâche
    /// qui en attend une autre, jamais dans le fil de l'interface.
    pub fn join(self) {
        let _ = self.thread.join();
    }
}

/// Un verrou empoisonné ne protège ici que des données qui restent cohérentes :
/// on continue plutôt que de propager la panique d'un autre fil.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn panic_text(panic: &Box<dyn Any + Send>) -> &str {
    if let Some(text) = panic.downcast_ref::<&str>() {
        text
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text
    } else {
        "cause inconnue"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    /// Ces types se partagent entre fils, comme le promet `docs/contrats.md`.
    #[test]
    fn shared_types_are_send_and_sync() {
        fn shareable<T: Send + Sync>() {}
        shareable::<TaskContext>();
        shareable::<Tasks>();
        shareable::<Publisher>();
        shareable::<CancelToken>();
    }

    fn task_states(bus: &Bus) -> Vec<TaskState> {
        bus.drain()
            .into_iter()
            .filter_map(|event| match event {
                Event::Task(event) => Some(event.state),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_task_publishes_start_results_and_end() {
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let handle = tasks.spawn(Domain::Index, "test.work", |ctx| {
            ctx.publish(String::from("résultat"));
            Ok(())
        });
        let id = handle.id();
        handle.join();

        let events = bus.drain();
        assert_eq!(events.len(), 3);
        assert!(matches!(
            &events[0],
            Event::Task(TaskEvent {
                state: TaskState::Started,
                kind: "test.work",
                domain: Domain::Index,
                ..
            })
        ));
        match &events[1] {
            Event::Domain(event) => {
                assert_eq!(event.domain, Domain::Index);
                assert_eq!(event.task, Some(id));
                assert_eq!(event.get::<String>().unwrap(), "résultat");
            }
            other => panic!("événement inattendu : {other:?}"),
        }
        assert!(matches!(
            &events[2],
            Event::Task(TaskEvent {
                state: TaskState::Finished(TaskOutcome::Done),
                ..
            })
        ));
        assert_eq!(tasks.running_count(), 0);
    }

    #[test]
    fn cancelling_stops_a_cooperative_task() {
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let handle = tasks.spawn(Domain::Vcs, "test.loop", |ctx| loop {
            ctx.check_cancelled()?;
            std::thread::sleep(Duration::from_millis(1));
        });
        assert_eq!(tasks.running_count(), 1);
        tasks.cancel(handle.id());
        handle.join();
        assert_eq!(
            task_states(&bus),
            vec![
                TaskState::Started,
                TaskState::Finished(TaskOutcome::Cancelled)
            ]
        );
    }

    #[test]
    fn cancel_all_reaches_every_running_task() {
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        let handles: Vec<_> = (0..3)
            .map(|_| {
                tasks.spawn(Domain::PhpUnit, "test.loop", |ctx| loop {
                    ctx.check_cancelled()?;
                    std::thread::sleep(Duration::from_millis(1));
                })
            })
            .collect();
        tasks.cancel_all();
        for handle in handles {
            handle.join();
        }
        assert_eq!(tasks.running_count(), 0);
    }

    #[test]
    fn errors_and_panics_become_failed_outcomes() {
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        tasks
            .spawn(Domain::Diff, "test.error", |_| {
                Err(Error::Invalid("cassé".into()).into())
            })
            .join();
        tasks
            .spawn(Domain::Diff, "test.panic", |_| panic!("boum"))
            .join();
        assert_eq!(
            task_states(&bus),
            vec![
                TaskState::Started,
                TaskState::Finished(TaskOutcome::Failed("cassé".into())),
                TaskState::Started,
                TaskState::Finished(TaskOutcome::Failed("panique : boum".into())),
            ]
        );
        assert_eq!(tasks.running_count(), 0);
    }

    #[test]
    fn progress_is_throttled_but_the_last_step_always_arrives() {
        let bus = Bus::new();
        let tasks = Tasks::new(bus.publisher());
        tasks
            .spawn(Domain::Index, "test.progress", |ctx| {
                for done in 1..=50_000u64 {
                    ctx.progress(done, 50_000);
                }
                Ok(())
            })
            .join();
        let progress: Vec<_> = task_states(&bus)
            .into_iter()
            .filter(|state| matches!(state, TaskState::Progress { .. }))
            .collect();
        assert_eq!(
            progress.first(),
            Some(&TaskState::Progress {
                done: 1,
                total: 50_000
            })
        );
        assert_eq!(
            progress.last(),
            Some(&TaskState::Progress {
                done: 50_000,
                total: 50_000
            })
        );
        assert!(
            progress.len() < 500,
            "{} événements de progression",
            progress.len()
        );
    }
}
