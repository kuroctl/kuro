//! « Ajouter un nœud » : nœud `pending` + token d'enrôlement à usage unique.
//! Utilisé à l'identique par `POST /api/admin/v1/nodes` et `kuro-api node add`.

use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::enrollment::domain::EnrollmentToken;
use crate::enrollment::infra::EnrollmentRepository;
use crate::nodes::application::{NodesError, PlacementInput, placement};
use crate::nodes::domain::{InvalidNodeName, NodeId, NodeName};
use crate::nodes::infra::{NodeRecord, NodeRepository};

#[derive(Debug, thiserror::Error)]
pub enum AddNodeError {
    #[error(transparent)]
    InvalidName(#[from] InvalidNodeName),
    #[error("a node with this name already exists")]
    AlreadyExists,
    #[error(transparent)]
    Placement(#[from] NodesError),
    #[error(transparent)]
    Random(#[from] kuro_runtime::tls::TlsError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Ce qu'il faut transmettre à l'opérateur de l'hôte à enrôler. Le token n'est montré qu'une fois.
#[derive(Debug)]
pub struct NodeTicket {
    pub node: NodeRecord,
    pub token: EnrollmentToken,
    pub expires_in: Duration,
}

pub async fn add_node(
    pool: &PgPool,
    context: &AuditContext,
    name: &str,
    placement_input: PlacementInput,
    ttl: Duration,
) -> Result<NodeTicket, AddNodeError> {
    let name = NodeName::parse(name)?;
    let token = EnrollmentToken::generate()?;
    let id = NodeId::new();

    let mut tx = AuditedTx::begin(pool, context).await?;
    let placement = placement::validate(&mut tx, placement_input).await?;
    NodeRepository::insert_pending(&mut tx, id, &name, &placement)
        .await
        .map_err(|error| match error.as_database_error() {
            Some(db) if db.is_unique_violation() => AddNodeError::AlreadyExists,
            _ => AddNodeError::Database(error),
        })?;
    EnrollmentRepository::insert(&mut tx, id, &token.hash(), ttl).await?;
    let node = NodeRepository::get(&mut tx, id.as_uuid())
        .await?
        .ok_or(NodesError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::NodeAdded)
            .target("node", id)
            .details(json!({
                "name": name.as_str(),
                "region_id": placement.region_id,
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
