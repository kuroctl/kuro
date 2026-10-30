//! Instance de l'API en cours (ADR-0037) : s'inscrit, bat la seconde, nettoie les instances
//! mortes. Une installation à une seule instance n'y voit qu'une ligne et un battement.

use std::time::Duration;

use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::nodes::domain::InstanceId;
use crate::nodes::infra::{InstanceRepository, Swept};

/// Période du battement.
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
/// Sans battement pendant ce délai, une instance est morte.
pub const DEAD_AFTER: Duration = Duration::from_secs(20);
/// Période du nettoyage des instances mortes.
pub const SWEEP_EVERY: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct Instance {
    pub id: InstanceId,
    repository: InstanceRepository,
}

impl Instance {
    /// Nouvelle instance, inscrite en base.
    pub async fn start(pool: PgPool) -> Result<Self, sqlx::Error> {
        let id = InstanceId::generate();
        let repository = InstanceRepository::new(pool);
        repository.register(id).await?;
        Ok(Self { id, repository })
    }

    /// Nettoie ce qu'ont laissé les instances mortes (au démarrage, puis périodiquement).
    pub async fn sweep(&self) -> Result<Swept, sqlx::Error> {
        let swept = self.repository.sweep(self.id, DEAD_AFTER).await?;
        // Nœuds passés hors ligne par le balayage : notifiés (ADR-0044).
        for (id, name) in &swept.offline {
            let event = crate::notifications::domain::Event::now(
                crate::notifications::domain::EventKind::NodeOffline,
            )
            .node(crate::notifications::domain::NodeRef {
                id: *id,
                name: name.clone(),
            });
            crate::notifications::application::emit_detached(self.repository.pool(), event);
        }
        Ok(swept)
    }

    /// Bat la seconde et balaie jusqu'à l'arrêt ; à l'arrêt, se désinscrit (ses nœuds passent
    /// `offline` sans attendre `DEAD_AFTER`).
    pub async fn run_forever(self, shutdown: CancellationToken) {
        let mut heartbeat = tokio::time::interval(HEARTBEAT_EVERY);
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut sweep = tokio::time::interval(SWEEP_EVERY);
        sweep.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = heartbeat.tick() => match self.repository.heartbeat(self.id).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(instance = %self.id, "instance row was swept, registering again");
                        if let Err(error) = self.repository.register(self.id).await {
                            tracing::error!(%error, "failed to register the instance again");
                        }
                    }
                    Err(error) => tracing::error!(%error, "instance heartbeat failed"),
                },
                _ = sweep.tick() => match self.sweep().await {
                    Ok(swept) if swept != Swept::default() => {
                        tracing::info!(instances = swept.instances, nodes = swept.nodes, "dead instances swept");
                    }
                    Ok(_) => {}
                    Err(error) => tracing::error!(%error, "instance sweep failed"),
                },
            }
        }
        if let Err(error) = self.repository.unregister(self.id).await {
            tracing::warn!(%error, "failed to unregister the instance");
        }
    }
}
