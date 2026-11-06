//! Sauvegardes gardées des serveurs supprimés (ADR-0059) : leur liste pour l'admin, et leur reprise
//! par un serveur du même nœud, qui peut ensuite les restaurer comme les siennes.

use kuro_protocol::ApiMessage;
use kuro_protocol::api::BackupAdopt;
use serde_json::json;
use uuid::Uuid;

use super::{ServersError, ServersState, get};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::ErrorCode;
use crate::nodes::domain::{NodeId, NodeStatus};
use crate::servers::infra::{DeletedBackupRecord, DeletedBackupRepository};

pub async fn list(state: &ServersState) -> Result<Vec<DeletedBackupRecord>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    Ok(DeletedBackupRepository::list(&mut conn, None).await?)
}

/// Reprend dans `target` les sauvegardes gardées du serveur supprimé `from` ; retourne leur
/// nombre. Le serveur doit être sur le même nœud (les archives n'en sortent pas), prêt, et ce
/// nœud en ligne.
pub async fn adopt(
    state: &ServersState,
    context: &AuditContext,
    from: Uuid,
    target: Uuid,
) -> Result<u64, ServersError> {
    let kept = {
        let mut conn = state.pool.acquire().await?;
        DeletedBackupRepository::list(&mut conn, Some(from)).await?
    };
    let node = kept
        .first()
        .ok_or(ServersError::Missing(ErrorCode::BackupNotFound))?
        .node_id;
    let server = get(state, target).await?.server;
    if server.node_id != node {
        return Err(ServersError::Conflict(ErrorCode::DeletedBackupsOtherNode));
    }
    super::manage::ensure_ready(&server)?;
    if server.node_status.parse::<NodeStatus>().ok() != Some(NodeStatus::Online) {
        return Err(ServersError::Conflict(ErrorCode::NodeOffline));
    }
    let result = state
        .commands
        .send(
            NodeId::from(node),
            ApiMessage::BackupAdopt(BackupAdopt {
                from_server_id: from,
                server_id: target,
            }),
        )
        .await?;
    super::deploy::accepted(result)?;

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let adopted = DeletedBackupRepository::adopt(&mut tx, from, target).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerBackupsAdopted)
            .target("server", target)
            .details(json!({ "from": from, "count": adopted })),
    )
    .await?;
    Ok(adopted)
}
