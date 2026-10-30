use std::time::Duration;

use sqlx::PgConnection;
use uuid::Uuid;

/// Jeton de rafraîchissement retrouvé par son hash, verrouillé pour la transaction.
#[derive(Debug)]
pub struct SessionRecord {
    pub id: Uuid,
    pub family_id: Uuid,
    pub user_id: Uuid,
    pub expired: bool,
    pub rotated: bool,
    /// Remplacé il y a moins que la fenêtre de grâce demandée.
    pub rotated_recently: bool,
    pub revoked: bool,
    pub user_disabled: bool,
    /// La session (sa famille de jetons) a dépassé sa durée de vie maximale.
    pub too_old: bool,
    /// Jeton qui a remplacé celui-ci, chiffré (rotation récente seulement).
    pub successor_secret: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct SessionRepository;

impl SessionRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        family_id: Uuid,
        user_id: Uuid,
        refresh_token_hash: &str,
        ttl: Duration,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO admin_sessions (id, family_id, user_id, refresh_token_hash, expires_at)
             VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))",
            Uuid::now_v7(),
            family_id,
            user_id,
            refresh_token_hash,
            ttl.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn find_for_update(
        conn: &mut PgConnection,
        refresh_token_hash: &str,
        rotation_grace: Duration,
        max_age: Duration,
    ) -> Result<Option<SessionRecord>, sqlx::Error> {
        sqlx::query_as!(
            SessionRecord,
            r#"SELECT s.id, s.family_id, s.user_id,
                      (s.expires_at <= now()) AS "expired!",
                      (s.rotated_at IS NOT NULL) AS "rotated!",
                      (s.rotated_at IS NOT NULL
                       AND s.rotated_at > now() - make_interval(secs => $2)) AS "rotated_recently!",
                      (s.revoked_at IS NOT NULL) AS "revoked!",
                      (u.disabled_at IS NOT NULL) AS "user_disabled!",
                      ((SELECT min(f.created_at) FROM admin_sessions f WHERE f.family_id = s.family_id)
                       <= now() - make_interval(secs => $3)) AS "too_old!",
                      s.successor_secret
               FROM admin_sessions s JOIN admin_users u ON u.id = s.user_id
               WHERE s.refresh_token_hash = $1
               FOR UPDATE OF s"#,
            refresh_token_hash,
            rotation_grace.as_secs_f64(),
            max_age.as_secs_f64(),
        )
        .fetch_optional(conn)
        .await
    }

    /// Jeton consommé ; son successeur est gardé chiffré pour la fenêtre de grâce.
    pub async fn mark_rotated(
        conn: &mut PgConnection,
        id: Uuid,
        successor_secret: &[u8],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_sessions SET rotated_at = now(), successor_secret = $2 WHERE id = $1",
            id,
            successor_secret,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Efface les successeurs dont la fenêtre de grâce est passée : ils ne servent plus.
    pub async fn forget_successors(
        conn: &mut PgConnection,
        rotation_grace: Duration,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE admin_sessions SET successor_secret = NULL
             WHERE successor_secret IS NOT NULL
               AND rotated_at <= now() - make_interval(secs => $1)",
            rotation_grace.as_secs_f64(),
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn revoke_family(
        conn: &mut PgConnection,
        family_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_sessions SET revoked_at = COALESCE(revoked_at, now()) WHERE family_id = $1",
            family_id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Révoque toutes les sessions du compte, sauf éventuellement `keep`.
    pub async fn revoke_user(
        conn: &mut PgConnection,
        user_id: Uuid,
        keep: Option<Uuid>,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "UPDATE admin_sessions SET revoked_at = now()
             WHERE user_id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR family_id <> $2)",
            user_id,
            keep,
        )
        .execute(conn)
        .await?;
        Ok(result.rows_affected())
    }
}
