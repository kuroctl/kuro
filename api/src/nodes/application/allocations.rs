//! Ports d'un nœud (ADR-0013) : créés par plages, assignés aux serveurs au déploiement.

use std::net::IpAddr;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::NodesError;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::infra::{AllocationRecord, AllocationRepository, NodeRepository};

/// Ports par requête, pour qu'une faute de frappe ne crée pas des dizaines de milliers d'entrées.
pub const MAX_PORTS_PER_REQUEST: usize = 1000;
/// Les ports privilégiés de l'hôte ne sont jamais alloués.
pub const MIN_PORT: u16 = 1024;
pub const DEFAULT_IP: &str = "0.0.0.0";

/// `25565`, `25565-25600` ou une liste `25565,25570-25575`.
pub fn parse_ports(spec: &str) -> Option<Vec<i32>> {
    let mut ports = Vec::new();
    for part in spec.split(',').map(str::trim) {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (
                start.trim().parse::<u16>().ok()?,
                end.trim().parse::<u16>().ok()?,
            ),
            None => {
                let port = part.parse::<u16>().ok()?;
                (port, port)
            }
        };
        if start < MIN_PORT || end < start {
            return None;
        }
        ports.extend((start..=end).map(i32::from));
        if ports.len() > MAX_PORTS_PER_REQUEST {
            return None;
        }
    }
    ports.sort_unstable();
    ports.dedup();
    (!ports.is_empty()).then_some(ports)
}

pub async fn list(pool: &PgPool, node_id: Uuid) -> Result<Vec<AllocationRecord>, NodesError> {
    let mut conn = pool.acquire().await?;
    NodeRepository::get(&mut conn, node_id)
        .await?
        .ok_or(NodesError::NotFound)?;
    Ok(AllocationRepository::list(&mut conn, node_id).await?)
}

/// Crée les ports absents de la plage ; retourne toutes les allocations du nœud.
pub async fn create(
    pool: &PgPool,
    context: &AuditContext,
    node_id: Uuid,
    ip: Option<&str>,
    ports: &str,
) -> Result<Vec<AllocationRecord>, NodesError> {
    let ip = ip
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
        .unwrap_or(DEFAULT_IP);
    let ip: IpAddr = ip
        .parse()
        .map_err(|_| NodesError::Invalid(FieldError::new("ip", ErrorCode::InvalidIp)))?;
    let ports = parse_ports(ports).ok_or_else(|| {
        NodesError::Invalid(
            FieldError::new("ports", ErrorCode::InvalidPortRange)
                .param("min", MIN_PORT)
                .param("max_count", MAX_PORTS_PER_REQUEST),
        )
    })?;

    let mut tx = AuditedTx::begin(pool, context).await?;
    NodeRepository::get(&mut tx, node_id)
        .await?
        .ok_or(NodesError::NotFound)?;
    let created =
        AllocationRepository::insert_many(&mut tx, node_id, &ip.to_string(), &ports).await?;
    let all = AllocationRepository::list(&mut tx, node_id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::AllocationsCreated)
            .target("node", node_id)
            .details(json!({
                "ip": ip.to_string(),
                "first": ports.first(),
                "last": ports.last(),
                "requested": ports.len(),
                "created": created,
            })),
    )
    .await?;
    Ok(all)
}

/// Supprime une allocation libre. Assignée, elle reste (le serveur l'utilise).
pub async fn delete(
    pool: &PgPool,
    context: &AuditContext,
    node_id: Uuid,
    id: Uuid,
) -> Result<(), NodesError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let allocation = AllocationRepository::get(&mut tx, node_id, id)
        .await?
        .ok_or(NodesError::NotFound)?;
    if allocation.server_id.is_some() || !AllocationRepository::delete_free(&mut tx, id).await? {
        return Err(NodesError::Conflict(ErrorCode::AllocationInUse));
    }
    tx.commit(
        AuditEvent::success(AuditAction::AllocationDeleted)
            .target("allocation", id)
            .details(json!({ "node_id": node_id, "ip": allocation.ip, "port": allocation.port })),
    )
    .await?;
    Ok(())
}
