//! Demandes de réinitialisation du mot de passe (ADR-0049) : seule l'empreinte du jeton est
//! stockée ; une demande consommée ou remplacée garde sa trace.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PasswordResetRecord {
    pub id: Uuid,
    pub user_id: Uuid,
}

#[derive(Debug)]
pub struct PasswordResetRepository;

impl PasswordResetRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        user_id: Uuid,
        token_hash: &str,
        expires_at: OffsetDateTime,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO admin_password_resets (id, user_id, token_hash, expires_at)
             VALUES ($1, $2, $3, $4)",
            id,
            user_id,
            token_hash,
            expires_at,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Une demande a été faite pour ce compte depuis moins de `secs` secondes.
    pub async fn requested_within(
        conn: &mut PgConnection,
        user_id: Uuid,
        secs: f64,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                   SELECT 1 FROM admin_password_resets
                   WHERE user_id = $1 AND created_at > now() - make_interval(secs => $2)
               ) AS "recent!""#,
            user_id,
            secs,
        )
        .fetch_one(conn)
        .await
    }

    /// Révoque la demande ouverte du compte, s'il y en a une.
    pub async fn revoke_open(conn: &mut PgConnection, user_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_password_resets SET revoked_at = now()
             WHERE user_id = $1 AND used_at IS NULL AND revoked_at IS NULL",
            user_id
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Demande encore valable portant cette empreinte, verrouillée pour la transaction.
    pub async fn open_by_hash(
        conn: &mut PgConnection,
        token_hash: &str,
    ) -> Result<Option<PasswordResetRecord>, sqlx::Error> {
        sqlx::query_as!(
            PasswordResetRecord,
            "SELECT id, user_id FROM admin_password_resets
             WHERE token_hash = $1 AND used_at IS NULL AND revoked_at IS NULL
               AND expires_at > now()
             FOR UPDATE",
            token_hash
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn mark_used(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_password_resets SET used_at = now() WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }
}
