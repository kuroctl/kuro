use sqlx::PgConnection;
use uuid::Uuid;

/// Codes de secours du TOTP, connus par leur seule empreinte.
#[derive(Debug)]
pub struct RecoveryCodeRepository;

impl RecoveryCodeRepository {
    /// Remplace tous les codes du compte (utilisés ou non) par ceux-ci.
    pub async fn replace(
        conn: &mut PgConnection,
        user_id: Uuid,
        hashes: &[Vec<u8>],
    ) -> Result<(), sqlx::Error> {
        Self::delete_all(conn, user_id).await?;
        sqlx::query!(
            "INSERT INTO admin_recovery_codes (user_id, code_hash) SELECT $1, unnest($2::bytea[])",
            user_id,
            hashes,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Consomme le code s'il est valide et inutilisé ; retourne le nombre de codes restants.
    pub async fn consume(
        conn: &mut PgConnection,
        user_id: Uuid,
        hash: &[u8],
    ) -> Result<Option<i64>, sqlx::Error> {
        let used = sqlx::query!(
            "UPDATE admin_recovery_codes SET used_at = now()
             WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
            user_id,
            hash,
        )
        .execute(&mut *conn)
        .await?
        .rows_affected();
        if used == 0 {
            return Ok(None);
        }
        Self::remaining(conn, user_id).await.map(Some)
    }

    pub async fn remaining(conn: &mut PgConnection, user_id: Uuid) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM admin_recovery_codes
               WHERE user_id = $1 AND used_at IS NULL"#,
            user_id,
        )
        .fetch_one(conn)
        .await
    }

    pub async fn delete_all(conn: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "DELETE FROM admin_recovery_codes WHERE user_id = $1",
            user_id
        )
        .execute(conn)
        .await?;
        Ok(())
    }
}
