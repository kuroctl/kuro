//! Invitations des admins (ADR-0046) : seule l'empreinte du jeton est stockée ; une invitation
//! consommée ou remplacée garde sa trace.

use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InvitationRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct InvitationRepository;

impl InvitationRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        user_id: Uuid,
        token_hash: &str,
        invited_by: Option<Uuid>,
        expires_at: OffsetDateTime,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO admin_invitations (id, user_id, token_hash, invited_by, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
            id,
            user_id,
            token_hash,
            invited_by,
            expires_at,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Révoque l'invitation ouverte du compte, s'il y en a une ; vrai si c'est le cas.
    pub async fn revoke_open(conn: &mut PgConnection, user_id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE admin_invitations SET revoked_at = now()
             WHERE user_id = $1 AND used_at IS NULL AND revoked_at IS NULL",
            user_id
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Invitation encore valable portant cette empreinte.
    pub async fn open_by_hash(
        conn: &mut PgConnection,
        token_hash: &str,
    ) -> Result<Option<InvitationRecord>, sqlx::Error> {
        sqlx::query_as!(
            InvitationRecord,
            "SELECT id, user_id, expires_at, created_at FROM admin_invitations
             WHERE token_hash = $1 AND used_at IS NULL AND revoked_at IS NULL
               AND expires_at > now()",
            token_hash
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn mark_used(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_invitations SET used_at = now() WHERE id = $1",
            id
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Comptes dont une invitation est en attente (ouverte et non expirée).
    pub async fn pending_user_ids(conn: &mut PgConnection) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT user_id FROM admin_invitations
             WHERE used_at IS NULL AND revoked_at IS NULL AND expires_at > now()"
        )
        .fetch_all(conn)
        .await
    }
}
