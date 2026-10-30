//! Sauvegardes stockées (ADR-0018) : état, taille et empreinte des archives gardées sur le nœud.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct BackupRecord {
    pub id: Uuid,
    pub server_id: Uuid,
    pub name: String,
    pub state: String,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub locked: bool,
    pub failure: Option<String>,
    pub created_at: OffsetDateTime,
    pub completed_at: Option<OffsetDateTime>,
}

#[derive(Debug)]
pub struct BackupRepository;

impl BackupRepository {
    /// Sauvegardes d'un serveur, de la plus récente à la plus ancienne.
    pub async fn of_server(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<BackupRecord>, sqlx::Error> {
        sqlx::query_as!(
            BackupRecord,
            "SELECT id, server_id, name, state, size_bytes, sha256, locked, failure, created_at,
                    completed_at
             FROM backups WHERE server_id = $1 ORDER BY created_at DESC, id DESC",
            server_id,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        server_id: Uuid,
        id: Uuid,
    ) -> Result<Option<BackupRecord>, sqlx::Error> {
        sqlx::query_as!(
            BackupRecord,
            "SELECT id, server_id, name, state, size_bytes, sha256, locked, failure, created_at,
                    completed_at
             FROM backups WHERE server_id = $1 AND id = $2",
            server_id,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        server_id: Uuid,
        name: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO backups (id, server_id, name, state) VALUES ($1, $2, $3, 'creating')",
            id,
            server_id,
            name,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Fin rapportée par le nœud du serveur, pour une sauvegarde encore en cours seulement.
    pub async fn complete(
        conn: &mut PgConnection,
        id: Uuid,
        node_id: Uuid,
        outcome: Result<(i64, &str), &str>,
    ) -> Result<bool, sqlx::Error> {
        let (state, size, sha256, failure) = match outcome {
            Ok((size, sha256)) => ("ready", Some(size), Some(sha256), None),
            Err(reason) => ("failed", None, None, Some(reason)),
        };
        let updated = sqlx::query!(
            "UPDATE backups b SET state = $3, size_bytes = $4, sha256 = $5, failure = $6,
                    completed_at = now()
             FROM servers s
             WHERE b.id = $1 AND b.server_id = s.id AND s.node_id = $2 AND b.state = 'creating'",
            id,
            node_id,
            state,
            size,
            sha256,
            failure,
        )
        .execute(conn)
        .await?;
        Ok(updated.rows_affected() == 1)
    }

    /// Sauvegardes en cours des serveurs du nœud : (serveur, sauvegarde).
    pub async fn creating_on_node(
        conn: &mut PgConnection,
        node_id: Uuid,
    ) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
        let rows = sqlx::query!(
            "SELECT b.server_id, b.id FROM backups b JOIN servers s ON s.id = b.server_id
             WHERE s.node_id = $1 AND b.state = 'creating'",
            node_id,
        )
        .fetch_all(conn)
        .await?;
        Ok(rows.into_iter().map(|r| (r.server_id, r.id)).collect())
    }

    pub async fn set_locked(
        conn: &mut PgConnection,
        id: Uuid,
        locked: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!("UPDATE backups SET locked = $2 WHERE id = $1", id, locked)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM backups WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }
}
