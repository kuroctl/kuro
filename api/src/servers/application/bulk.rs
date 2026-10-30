//! Actions groupées sur une sélection de serveurs (ADR-0025). Chaque serveur passe par le cas
//! d'usage habituel, avec ses vérifications et son événement d'audit ; un refus n'arrête pas
//! les autres. Quelques serveurs à la fois seulement : le nœud répond à chaque commande.

use futures_util::StreamExt;
use futures_util::stream;
use kuro_protocol::api::server::PowerAction;
use uuid::Uuid;

use super::search::{latest_versions, newer};
use super::{ServersState, backups, get, power, scheduler, set_suspended, upgrade};
use crate::audit::domain::AuditContext;
use crate::error_code::ErrorCode;
use crate::servers::infra::ServerIndexRow;

/// Serveurs traités au plus par action groupée.
pub const MAX_SERVERS: usize = 1000;
/// Serveurs traités en même temps.
const CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkAction {
    Power(PowerAction),
    /// Vers la dernière version de leur prefab ; `backup` : sauvegarde attendue d'abord.
    Upgrade {
        backup: bool,
    },
    Backup,
    Suspend,
    Unsuspend,
}

/// Issue pour un serveur : `error` est un code stable (`server_busy`, `upgrade_blocked`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub server_id: Uuid,
    pub error: Option<String>,
}

pub async fn run(
    state: &ServersState,
    context: &AuditContext,
    action: BulkAction,
    ids: &[Uuid],
) -> Vec<Outcome> {
    let latest = match action {
        BulkAction::Upgrade { .. } => latest_versions(state).await.unwrap_or_default(),
        _ => Default::default(),
    };
    let latest = &latest;
    stream::iter(ids.iter().copied())
        .map(|id| async move {
            let error = one(state, context, action, latest, id)
                .await
                .err()
                .map(|error| scheduler::reason_code(&error));
            Outcome {
                server_id: id,
                error,
            }
        })
        .buffered(CONCURRENCY)
        .collect()
        .await
}

async fn one(
    state: &ServersState,
    context: &AuditContext,
    action: BulkAction,
    latest: &std::collections::HashMap<String, String>,
    id: Uuid,
) -> Result<(), super::ServersError> {
    match action {
        BulkAction::Power(action) => power(state, context, id, action).await,
        BulkAction::Backup => backups::create(state, context, id, None).await.map(|_| ()),
        BulkAction::Suspend => set_suspended(state, context, id, true).await.map(|_| ()),
        BulkAction::Unsuspend => set_suspended(state, context, id, false).await.map(|_| ()),
        BulkAction::Upgrade { backup } => {
            let server = get(state, id).await?.server;
            let row = ServerIndexRow {
                id,
                name: server.name.clone(),
                node_id: server.node_id,
                node_name: server.node_name.clone(),
                prefab_id: server.prefab_id.clone(),
                prefab_version: server.prefab_version.clone(),
                state: server.state.clone(),
                suspended: server.suspended_at.is_some(),
                memory_mb: server.memory_mb,
                cpu_percent: server.cpu_percent,
                disk_mb: server.disk_mb,
                created_at: server.created_at,
            };
            let to = newer(latest, &row).ok_or(super::ServersError::Unprocessable(
                ErrorCode::UpgradeNotNewer,
            ))?;
            upgrade::start(state, context, id, to, backup)
                .await
                .map(|_| ())
        }
    }
}
