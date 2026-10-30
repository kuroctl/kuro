//! Nouveau token pour un nœud `pending` (token perdu ou expiré) ou `revoked` (réenrôlement après
//! révocation : le nœud repasse `pending`, son ancien certificat est oublié). Les tokens non
//! utilisés du nœud sont invalidés : un seul token valide à la fois.

use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::{AddNodeError, NodeTicket};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::enrollment::domain::EnrollmentToken;
use crate::enrollment::infra::EnrollmentRepository;
use crate::error_code::ErrorCode;
use crate::nodes::application::NodesError;
use crate::nodes::domain::{NodeId, NodeStatus};
use crate::nodes::infra::NodeRepository;

pub async fn reissue_token(
    pool: &PgPool,
    context: &AuditContext,
    node_id: Uuid,
    ttl: Duration,
) -> Result<NodeTicket, AddNodeError> {
    let token = EnrollmentToken::generate()?;
    let id = NodeId::from(node_id);

    let mut tx = AuditedTx::begin(pool, context).await?;
    let status = NodeRepository::lock_status(&mut tx, node_id)
        .await?
        .ok_or(NodesError::NotFound)?;
    let reenrollment = match status {
        NodeStatus::Pending => false,
        NodeStatus::Revoked => {
            NodeRepository::reset_for_enrollment(&mut tx, node_id).await?;
            true
        }
        NodeStatus::Online | NodeStatus::Offline => {
            return Err(NodesError::Conflict(ErrorCode::NodeNotPending).into());
        }
    };
    let invalidated = EnrollmentRepository::expire_unused(&mut tx, id).await?;
    EnrollmentRepository::insert(&mut tx, id, &token.hash(), ttl).await?;
    let node = NodeRepository::get(&mut tx, node_id)
        .await?
        .ok_or(NodesError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::NodeEnrollmentReissued)
            .target("node", node_id)
            .details(json!({
                "invalidated_tokens": invalidated,
                "reenrollment": reenrollment,
                "token_ttl_secs": ttl.as_secs(),
            })),
    )
    .await?;

    Ok(NodeTicket {
        node,
        token,
        expires_in: ttl,
    })
}
