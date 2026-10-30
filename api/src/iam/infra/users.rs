use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::iam::domain::SUPER_ADMIN_ROLE_ID;

/// Données d'authentification d'un compte, verrouillé pour la transaction (`FOR UPDATE`).
#[derive(Debug)]
pub struct Credentials {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub totp_secret: Option<Vec<u8>>,
    pub totp_pending_secret: Option<Vec<u8>>,
    pub totp_last_step: Option<i64>,
    /// Mot de passe choisi par un autre admin : à remplacer avant toute session.
    pub password_change_required: bool,
    pub locked: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct AdminUserRecord {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub totp_enabled: bool,
    pub disabled: bool,
    pub password_change_required: bool,
    pub last_login_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Debug)]
pub struct UserRepository;

impl UserRepository {
    pub async fn insert(
        conn: &mut PgConnection,
        id: Uuid,
        email: &str,
        display_name: &str,
        password_hash: &str,
        password_change_required: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO admin_users (id, email, display_name, password_hash, password_change_required)
             VALUES ($1, $2, $3, $4, $5)",
            id,
            email,
            display_name,
            password_hash,
            password_change_required,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn credentials_by_email(
        conn: &mut PgConnection,
        email: &str,
    ) -> Result<Option<Credentials>, sqlx::Error> {
        sqlx::query_as!(
            Credentials,
            r#"SELECT id, email, password_hash, totp_secret, totp_pending_secret, totp_last_step,
                      password_change_required,
                      (locked_until IS NOT NULL AND locked_until > now()) AS "locked!",
                      (disabled_at IS NOT NULL) AS "disabled!"
               FROM admin_users WHERE lower(email) = lower($1) FOR UPDATE"#,
            email,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn credentials_by_id(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<Credentials>, sqlx::Error> {
        sqlx::query_as!(
            Credentials,
            r#"SELECT id, email, password_hash, totp_secret, totp_pending_secret, totp_last_step,
                      password_change_required,
                      (locked_until IS NOT NULL AND locked_until > now()) AS "locked!",
                      (disabled_at IS NOT NULL) AS "disabled!"
               FROM admin_users WHERE id = $1 FOR UPDATE"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    /// Échec d'authentification. Retourne `true` si le compte vient d'être verrouillé.
    pub async fn record_failure(
        conn: &mut PgConnection,
        id: Uuid,
        max_attempts: i32,
        lock_secs: f64,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!(
            r#"UPDATE admin_users
               SET failed_attempts = CASE WHEN failed_attempts + 1 >= $2 THEN 0 ELSE failed_attempts + 1 END,
                   locked_until = CASE WHEN failed_attempts + 1 >= $2
                                       THEN now() + make_interval(secs => $3) ELSE locked_until END
               WHERE id = $1
               RETURNING (locked_until IS NOT NULL AND locked_until > now()) AS "locked!""#,
            id,
            max_attempts,
            lock_secs,
        )
        .fetch_one(conn)
        .await
    }

    /// Authentification réussie : compteurs remis à zéro, dernier pas TOTP mémorisé.
    pub async fn record_success(
        conn: &mut PgConnection,
        id: Uuid,
        totp_step: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users
             SET failed_attempts = 0, locked_until = NULL, last_login_at = now(),
                 totp_last_step = COALESCE($2, totp_last_step)
             WHERE id = $1",
            id,
            totp_step,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn set_pending_totp(
        conn: &mut PgConnection,
        id: Uuid,
        sealed_secret: &[u8],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users SET totp_pending_secret = $2, updated_at = now() WHERE id = $1",
            id,
            sealed_secret,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn enable_totp(
        conn: &mut PgConnection,
        id: Uuid,
        step: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users
             SET totp_secret = totp_pending_secret, totp_pending_secret = NULL,
                 totp_last_step = $2, updated_at = now()
             WHERE id = $1",
            id,
            step,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn reset_totp(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users
             SET totp_secret = NULL, totp_pending_secret = NULL, totp_last_step = NULL, updated_at = now()
             WHERE id = $1",
            id,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Nouveau mot de passe ; le verrouillage éventuel est levé. `change_required` : choisi par un
    /// autre admin, à remplacer à la prochaine connexion.
    pub async fn set_password(
        conn: &mut PgConnection,
        id: Uuid,
        password_hash: &str,
        change_required: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users
             SET password_hash = $2, password_change_required = $3, failed_attempts = 0,
                 locked_until = NULL, updated_at = now()
             WHERE id = $1",
            id,
            password_hash,
            change_required,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn update_profile(
        conn: &mut PgConnection,
        id: Uuid,
        email: Option<&str>,
        display_name: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users
             SET email = COALESCE($2, email), display_name = COALESCE($3, display_name), updated_at = now()
             WHERE id = $1",
            id,
            email,
            display_name,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn set_disabled(
        conn: &mut PgConnection,
        id: Uuid,
        disabled: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE admin_users
             SET disabled_at = CASE WHEN $2 THEN COALESCE(disabled_at, now()) ELSE NULL END, updated_at = now()
             WHERE id = $1",
            id,
            disabled,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    pub async fn delete(conn: &mut PgConnection, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM admin_users WHERE id = $1", id)
            .execute(conn)
            .await?;
        Ok(())
    }

    pub async fn id_by_email(
        conn: &mut PgConnection,
        email: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT id FROM admin_users WHERE lower(email) = lower($1)",
            email
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn get(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Option<AdminUserRecord>, sqlx::Error> {
        sqlx::query_as!(
            AdminUserRecord,
            r#"SELECT id, email, display_name, (totp_secret IS NOT NULL) AS "totp_enabled!",
                      (disabled_at IS NOT NULL) AS "disabled!", password_change_required,
                      last_login_at, created_at
               FROM admin_users WHERE id = $1"#,
            id,
        )
        .fetch_optional(conn)
        .await
    }

    pub async fn list(conn: &mut PgConnection) -> Result<Vec<AdminUserRecord>, sqlx::Error> {
        sqlx::query_as!(
            AdminUserRecord,
            r#"SELECT id, email, display_name, (totp_secret IS NOT NULL) AS "totp_enabled!",
                      (disabled_at IS NOT NULL) AS "disabled!", password_change_required,
                      last_login_at, created_at
               FROM admin_users ORDER BY created_at"#,
        )
        .fetch_all(conn)
        .await
    }

    /// Clés de permission accumulées par les rôles du compte.
    pub async fn permission_keys(
        conn: &mut PgConnection,
        id: Uuid,
    ) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT DISTINCT rp.permission FROM admin_user_roles ur
             JOIN role_permissions rp ON rp.role_id = ur.role_id
             WHERE ur.user_id = $1",
            id,
        )
        .fetch_all(conn)
        .await
    }

    pub async fn role_ids(conn: &mut PgConnection, id: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
        sqlx::query_scalar!(
            "SELECT role_id FROM admin_user_roles WHERE user_id = $1",
            id
        )
        .fetch_all(conn)
        .await
    }

    pub async fn set_roles(
        conn: &mut PgConnection,
        id: Uuid,
        role_ids: &[Uuid],
    ) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM admin_user_roles WHERE user_id = $1", id)
            .execute(&mut *conn)
            .await?;
        sqlx::query!(
            "INSERT INTO admin_user_roles (user_id, role_id) SELECT $1, unnest($2::uuid[])",
            id,
            role_ids,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// Super-admins actifs autres que `excluded` (règle du dernier super-admin).
    pub async fn other_active_super_admins(
        conn: &mut PgConnection,
        excluded: Uuid,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "count!" FROM admin_users u
               JOIN admin_user_roles ur ON ur.user_id = u.id
               WHERE ur.role_id = $1 AND u.disabled_at IS NULL AND u.id <> $2"#,
            SUPER_ADMIN_ROLE_ID,
            excluded,
        )
        .fetch_one(conn)
        .await
    }

    /// Le compte est actif et la session `session` ouverte (validation d'un jeton d'accès).
    pub async fn has_active_session(
        conn: &mut PgConnection,
        id: Uuid,
        session: Uuid,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar!(
            r#"SELECT EXISTS (
                 SELECT 1 FROM admin_sessions s JOIN admin_users u ON u.id = s.user_id
                 WHERE s.family_id = $2 AND s.user_id = $1 AND s.revoked_at IS NULL
                   AND u.disabled_at IS NULL
               ) AS "active!""#,
            id,
            session,
        )
        .fetch_one(conn)
        .await
    }
}
