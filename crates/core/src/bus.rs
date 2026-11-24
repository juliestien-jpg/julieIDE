//! Bus d'événements : le seul lien entre les crates métier et l'interface.
//!
//! Les tâches de fond gardent un [`Publisher`] (clonable, utilisable depuis
//! n'importe quel fil) ; l'interface vide le [`Bus`] une fois par image.
//!
//! egui ne redessine que sur demande : `app` injecte donc un rappel de réveil avec
//! [`Bus::set_waker`], appelé après chaque publication. Le noyau ne sait pas ce que
//! fait ce rappel.

use std::any::Any;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::event::{Domain, DomainEvent, Event};

type Waker = Box<dyn Fn() + Send + Sync>;

/// Côté réception, détenu par l'interface.
pub struct Bus {
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    waker: Arc<OnceLock<Waker>>,
}

/// Côté émission, à cloner dans chaque tâche de fond.
#[derive(Clone)]
pub struct Publisher {
    sender: Sender<Event>,
    waker: Arc<OnceLock<Waker>>,
}

impl Bus {
    pub fn new() -> Self {
        let (sender, receiver) = channel();
        Self {
            sender,
            receiver,
            waker: Arc::new(OnceLock::new()),
        }
    }

    pub fn publisher(&self) -> Publisher {
        Publisher {
            sender: self.sender.clone(),
            waker: Arc::clone(&self.waker),
        }
    }

    /// Installe le rappel appelé après chaque publication, depuis le fil qui
    /// publie : il doit être rapide et sûr entre fils (`egui::Context::request_repaint`
    /// l'est). Vaut pour tous les `Publisher`, déjà créés ou à venir. Ne s'installe
    /// qu'une fois : rend `false`, sans rien changer, si un rappel est déjà en place.
    pub fn set_waker(&self, waker: impl Fn() + Send + Sync + 'static) -> bool {
        self.waker.set(Box::new(waker)).is_ok()
    }

    /// Rend tous les événements en attente, dans l'ordre d'émission, sans bloquer.
    pub fn drain(&self) -> Vec<Event> {
        self.receiver.try_iter().collect()
    }

    /// Attend le prochain événement, au plus `timeout`. **Pour les tests** des
    /// crates métier ; jamais dans le fil de l'interface.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Event> {
        self.receiver.recv_timeout(timeout).ok()
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Publisher {
    /// Publie un événement puis réveille l'interface. Si l'interface a disparu,
    /// l'événement est perdu sans bruit : une tâche de fond n'a pas à planter parce
    /// que la fenêtre se ferme.
    pub fn publish(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            if let Some(wake) = self.waker.get() {
                wake();
            }
        }
    }

    /// Publie la charge utile d'une crate métier (voir [`DomainEvent`]).
    pub fn publish_domain<T: Any + Send + Sync>(&self, domain: Domain, payload: T) {
        self.publish(Event::Domain(DomainEvent::new(domain, payload)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn events_arrive_in_order_from_another_thread() {
        let bus = Bus::new();
        let publisher = bus.publisher();
        std::thread::spawn(move || {
            publisher.publish_domain(Domain::Vcs, 1u32);
            publisher.publish_domain(Domain::Index, 2u32);
        })
        .join()
        .unwrap();

        let received: Vec<(Domain, u32)> = bus
            .drain()
            .into_iter()
            .map(|event| match event {
                Event::Domain(event) => (event.domain, *event.get::<u32>().unwrap()),
                other => panic!("événement inattendu : {other:?}"),
            })
            .collect();
        assert_eq!(received, vec![(Domain::Vcs, 1), (Domain::Index, 2)]);
        assert!(bus.drain().is_empty());
    }

    #[test]
    fn publishing_after_the_bus_is_gone_does_not_panic() {
        let publisher = Bus::new().publisher();
        publisher.publish(Event::FilesChangedOnDisk(Vec::new()));
    }

    #[test]
    fn waker_is_called_once_per_publication_even_for_earlier_publishers() {
        let bus = Bus::new();
        let before = bus.publisher();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        assert!(bus.set_waker(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(!bus.set_waker(|| panic!("le second rappel ne doit pas être installé")));

        before.publish_domain(Domain::Text, ());
        bus.publisher().publish_domain(Domain::Text, ());
        assert_eq!(wakes.load(Ordering::SeqCst), 2);
        assert_eq!(bus.drain().len(), 2);
    }
}
