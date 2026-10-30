//! État des alertes de stats (ADR-0043, amendement) : une ligne par alerte active d'un serveur.

use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug)]
pub struct AlertRepository;

impl AlertRepository {
    /// Pose l'alerte ; vrai si elle n'était pas déjà active.
    pub async fn raise(
        conn: &mut PgConnection,
        server_id: Uuid,
        kind: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "INSERT INTO server_alerts (server_id, kind) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            server_id,
            kind
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Retire l'alerte ; vrai si elle était active.
    pub async fn clear(
        conn: &mut PgConnection,
        server_id: Uuid,
        kind: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM server_alerts WHERE server_id = $1 AND kind = $2",
            server_id,
            kind
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Alertes actives d'un serveur, par type.
    pub async fn active(
        conn: &mut PgConnection,
        server_id: Uuid,
    ) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT kind FROM server_alerts WHERE server_id = $1 ORDER BY kind",
            server_id
        )
        .fetch_all(conn)
        .await
    }
}
