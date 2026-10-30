use std::time::Duration;

use sqlx::PgConnection;
use uuid::Uuid;

use crate::enrollment::domain::TokenHash;
use crate::nodes::domain::NodeId;

/// Persistance des tokens (table `enrollment_tokens`), toujours dans une transaction.
#[derive(Debug)]
pub struct EnrollmentRepository;

/// Token valide verrouillé par la transaction courante.
#[derive(Debug)]
pub struct ClaimedToken {
    pub id: Uuid,
    pub node_id: NodeId,
}

impl EnrollmentRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        node_id: NodeId,
        hash: &TokenHash,
        ttl: Duration,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO enrollment_tokens (id, node_id, token_hash, expires_at)
             VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
            Uuid::now_v7(),
            node_id.as_uuid(),
            hash.as_str(),
            ttl.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Verrouille le token s'il est encore utilisable. Une transaction concurrente sur le même
    /// token attend ce verrou, puis ne le trouve plus (`used_at` renseigné).
    pub async fn claim(
        conn: &mut PgConnection,
        hash: &TokenHash,
    ) -> Result<Option<ClaimedToken>, sqlx::Error> {
        let row = sqlx::query!(
            "SELECT id, node_id FROM enrollment_tokens
             WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
             FOR UPDATE",
            hash.as_str(),
        )
        .fetch_optional(conn)
        .await?;
        Ok(row.map(|row| ClaimedToken {
            id: row.id,
            node_id: NodeId::from(row.node_id),
        }))
    }

    /// Expire les tokens non utilisés du nœud. Retourne leur nombre.
    pub async fn expire_unused(
        conn: &mut PgConnection,
        node_id: NodeId,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE enrollment_tokens SET expires_at = now()
             WHERE node_id = $1 AND used_at IS NULL AND expires_at > now()",
            node_id.as_uuid(),
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn mark_used(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE enrollment_tokens SET used_at = now() WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }
}
