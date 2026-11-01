//! Élévations des sessions admin (ADR-0050), une par famille de jetons.

use sqlx::PgConnection;
use uuid::Uuid;

#[derive(Debug)]
pub struct ElevationRepository;

impl ElevationRepository {
    /// Élève la session pour `secs` secondes, en remplaçant une élévation antérieure.
    pub async fn grant(
        conn: &mut PgConnection,
        family_id: Uuid,
        user_id: Uuid,
        secs: f64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "INSERT INTO admin_session_elevations (family_id, user_id, elevated_until)
             VALUES ($1, $2, now() + make_interval(secs => $3))
             ON CONFLICT (family_id) DO UPDATE SET elevated_until = EXCLUDED.elevated_until",
            family_id,
            user_id,
            secs,
        )
        .execute(conn)
        .await?;
        Ok(())
    }

    /// État de la session : élevée ou non, et TOTP actif ou non (la méthode à demander).
    pub async fn status(
        conn: &mut PgConnection,
        family_id: Uuid,
        user_id: Uuid,
    ) -> Result<(bool, bool), sqlx::Error> {
        let row = sqlx::query!(
            r#"SELECT
                   EXISTS (SELECT 1 FROM admin_session_elevations
                           WHERE family_id = $1 AND user_id = $2 AND elevated_until > now())
                       AS "elevated!",
                   (SELECT totp_secret IS NOT NULL FROM admin_users WHERE id = $2) AS "totp""#,
            family_id,
            user_id,
        )
        .fetch_one(conn)
        .await?;
        Ok((row.elevated, row.totp.unwrap_or(false)))
    }
}
