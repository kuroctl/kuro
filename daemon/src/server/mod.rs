//! Serveurs du nœud : règles du daemon sur les spécifications reçues et exécution des
//! commandes de l'API (ADR-0013).

mod manager;
pub mod spec;

pub use manager::{CommandError, Manager, Receivers, StateReceiver};

/// Gestion des serveurs branchée sur le lien : commandes, et messages à transmettre.
#[derive(Debug)]
pub struct Servers {
    pub manager: Manager,
    pub outgoing: Receivers,
}

impl Servers {
    /// Lien perdu : plus personne ne regarde. La console et les stats en attente sont jetées.
    pub fn forget_watches(&mut self) {
        self.manager.unwatch_all();
        while self.outgoing.streams.try_recv().is_ok() {}
    }
}
