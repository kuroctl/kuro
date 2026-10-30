//! États des serveurs à transmettre à l'API (`server.state`), coalescés par serveur : seul le
//! dernier état d'un serveur compte, et aucun n'est perdu quand le lien est coupé ou lent. La
//! file est bornée par le nombre de serveurs du nœud, pas par un tampon.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use kuro_protocol::daemon::server::ServerStateReport;
use tokio::sync::Notify;

#[derive(Debug, Default)]
struct Shared {
    /// Au plus un rapport par serveur, dans l'ordre de première apparition.
    pending: Mutex<VecDeque<ServerStateReport>>,
    notify: Notify,
}

/// Côté gestionnaire : dépose un état.
#[derive(Debug, Clone)]
pub struct StateSender {
    shared: Arc<Shared>,
}

/// Côté lien : lit les états à envoyer.
#[derive(Debug)]
pub struct StateReceiver {
    shared: Arc<Shared>,
}

pub fn channel() -> (StateSender, StateReceiver) {
    let shared = Arc::new(Shared::default());
    (
        StateSender {
            shared: Arc::clone(&shared),
        },
        StateReceiver { shared },
    )
}

impl StateSender {
    /// Remplace l'état en attente du même serveur, s'il y en a un.
    pub fn send(&self, report: ServerStateReport) {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match pending
            .iter_mut()
            .find(|queued| queued.server_id == report.server_id)
        {
            Some(queued) => *queued = report,
            None => pending.push_back(report),
        }
        drop(pending);
        self.shared.notify.notify_one();
    }
}

impl StateReceiver {
    /// Prochain état, dès qu'il y en a un. Toujours `Some` : il n'y a pas de fermeture.
    pub async fn recv(&mut self) -> Option<ServerStateReport> {
        loop {
            if let Some(report) = self.try_recv() {
                return Some(report);
            }
            self.shared.notify.notified().await;
        }
    }

    pub fn try_recv(&mut self) -> Option<ServerStateReport> {
        self.shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }
}

#[cfg(test)]
mod tests {
    use kuro_protocol::daemon::server::ServerState;
    use uuid::Uuid;

    use super::*;

    fn report(server_id: Uuid, state: ServerState) -> ServerStateReport {
        ServerStateReport {
            server_id,
            state,
            reason: None,
        }
    }

    #[tokio::test]
    async fn only_the_last_state_of_a_server_is_kept_and_none_is_lost() {
        let (sender, mut receiver) = channel();
        let servers: Vec<Uuid> = (0..300).map(|_| Uuid::now_v7()).collect();
        // Un redémarrage de masse : des milliers d'états, le lien coupé.
        for _ in 0..10 {
            for &server in &servers {
                sender.send(report(server, ServerState::Starting));
            }
        }
        for &server in &servers {
            sender.send(report(server, ServerState::Running));
        }

        let mut received = Vec::new();
        while let Some(state) = receiver.try_recv() {
            received.push(state);
        }
        assert_eq!(received.len(), servers.len());
        assert!(received.iter().all(|r| r.state == ServerState::Running));
        assert_eq!(
            received.iter().map(|r| r.server_id).collect::<Vec<_>>(),
            servers
        );
    }

    #[tokio::test]
    async fn a_reader_wakes_up_on_the_next_state() {
        let (sender, mut receiver) = channel();
        let server = Uuid::now_v7();
        let reader = tokio::spawn(async move { receiver.recv().await });
        tokio::task::yield_now().await;
        sender.send(report(server, ServerState::Offline));
        let received = tokio::time::timeout(std::time::Duration::from_secs(1), reader)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(received.server_id, server);
    }
}
