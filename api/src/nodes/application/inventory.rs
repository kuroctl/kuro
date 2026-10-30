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
