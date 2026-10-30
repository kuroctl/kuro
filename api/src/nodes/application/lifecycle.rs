//! Révocation et suppression d'un nœud. La session éventuelle est coupée **après** la validation
//! de la transaction : jamais de coupure pour une décision annulée.

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::NodesError;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::{DisconnectReason, NodeId, NodeLinks, NodeStatus};
use crate::nodes::infra::{NodeRecord, NodeRepository};

/// Le certificat du nœud est refusé dès maintenant ; un nouveau token permet de le réenrôler.
pub async fn revoke(
    pool: &PgPool,
    context: &AuditContext,
    links: &dyn NodeLinks,
    id: Uuid,
    reason: Option<String>,
) -> Result<NodeRecord, NodesError> {
    let reason = reason
        .map(|reason| reason.trim().to_owned())
        .filter(|reason| !reason.is_empty());
    if reason
        .as_deref()
        .is_some_and(|reason| !crate::validation::is_display_text(reason, 1, 500))
    {
        return Err(NodesError::Invalid(FieldError::new(
            "reason",
            ErrorCode::InvalidReason,
        )));
    }

    let mut tx = AuditedTx::begin(pool, context).await?;
    let status = NodeRepository::lock_status(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    match status {
        NodeStatus::Revoked => return Err(NodesError::Conflict(ErrorCode::NodeAlreadyRevoked)),
        // Sans certificat, il n'y a rien à révoquer : supprimer le nœud, ou attendre.
        NodeStatus::Pending => return Err(NodesError::Conflict(ErrorCode::NodeNotEnrolled)),
        NodeStatus::Online | NodeStatus::Offline => {}
    }
    NodeRepository::revoke(&mut tx, id).await?;
    let node = NodeRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::NodeRevoked)
            .target("node", id)
            .details(json!({ "reason": reason, "previous_status": status.as_str() })),
    )
    .await?;

    links.disconnect(NodeId::from(id), DisconnectReason::Revoked);
    Ok(node)
}

/// Possible dans tous les états. Les tokens partent avec le nœud ; le journal d'audit reste.
pub async fn delete(
    pool: &PgPool,
    context: &AuditContext,
    links: &dyn NodeLinks,
    id: Uuid,
) -> Result<(), NodesError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let node = NodeRepository::get(&mut tx, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    NodeRepository::delete(&mut tx, id)
        .await
        .map_err(|error| match error.as_database_error() {
            // Un serveur est encore déployé sur ce nœud (ADR-0013).
            Some(db) if db.is_foreign_key_violation() => {
                NodesError::Conflict(ErrorCode::NodeHasServers)
            }
            _ => error.into(),
        })?;
    tx.commit(
        AuditEvent::success(AuditAction::NodeDeleted)
            .target("node", id)
            .details(json!({ "name": node.name, "status": node.status })),
    )
    .await?;

    links.disconnect(NodeId::from(id), DisconnectReason::Deleted);
    Ok(())
}
