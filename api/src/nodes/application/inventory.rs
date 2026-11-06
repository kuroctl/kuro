//! Lecture des nœuds pour l'administration.

use sqlx::PgPool;
use uuid::Uuid;

use super::NodesError;
use crate::nodes::infra::{NodeRecord, NodeRepository, RegionFilter};

pub async fn list(pool: &PgPool, filter: RegionFilter) -> Result<Vec<NodeRecord>, NodesError> {
    let mut conn = pool.acquire().await?;
    Ok(NodeRepository::list(&mut conn, filter).await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<NodeRecord, NodesError> {
    let mut conn = pool.acquire().await?;
    NodeRepository::get(&mut conn, id)
        .await?
        .ok_or(NodesError::NotFound)
}

pub async fn id_by_name(pool: &PgPool, name: &str) -> Result<Uuid, NodesError> {
    let mut conn = pool.acquire().await?;
    NodeRepository::id_by_name(&mut conn, name)
        .await?
        .ok_or(NodesError::NotFound)
}

/// Place d'un nœud : ce qu'il annonce et ce que ses serveurs se sont déjà vu allouer.
#[derive(Debug, Clone, PartialEq, serde::Serialize, utoipa::ToSchema)]
pub struct NodeCapacity {
    pub node_id: Uuid,
    pub region_id: Option<Uuid>,
    /// `pending`, `online`, `offline`, `revoked`.
    pub status: String,
    /// Réservé (ADR-0065) : jamais choisi d'office.
    pub reserved: bool,
    /// Annoncés par le daemon ; absents tant qu'il ne s'est jamais connecté.
    pub cpu_cores: Option<i32>,
    pub memory_bytes: Option<i64>,
    /// Somme des limites de ses serveurs (100 = un cœur), tous états compris, y compris ceux qui
    /// attendent le nœud (`waiting_node`) et ceux en cours de suppression.
    pub allocated_cpu_percent: i64,
    pub allocated_memory_mb: i64,
    pub servers: i64,
}

/// Place de chaque nœud, filtrable par région, pour choisir où créer un serveur.
pub async fn capacity(
    pool: &PgPool,
    filter: RegionFilter,
) -> Result<Vec<NodeCapacity>, NodesError> {
    let (region, unassigned) = match filter {
        RegionFilter::All => (None, false),
        RegionFilter::Region(id) => (Some(id), false),
        RegionFilter::Unassigned => (None, true),
    };
    Ok(sqlx::query_as!(
        NodeCapacity,
        r#"SELECT n.id AS node_id, n.region_id, n.status, n.reserved, n.cpu_cores, n.memory_bytes,
                  COALESCE(SUM(s.cpu_percent), 0)::bigint AS "allocated_cpu_percent!",
                  COALESCE(SUM(s.memory_mb), 0)::bigint AS "allocated_memory_mb!",
                  COUNT(s.id) AS "servers!"
             FROM nodes n LEFT JOIN servers s ON s.node_id = n.id
            WHERE ($1::uuid IS NULL OR n.region_id = $1)
              AND (NOT $2 OR n.region_id IS NULL)
            GROUP BY n.id
            ORDER BY n.name"#,
        region,
        unassigned,
    )
    .fetch_all(pool)
    .await?)
}
