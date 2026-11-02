//! Tickets à usage unique de console et de téléchargement (ADR-0014, ADR-0018), en base pour
//! être consommables par n'importe quelle instance (ADR-0037). Seul le haché est gardé.

use std::time::Duration;

use sqlx::PgConnection;
use uuid::Uuid;

/// Ticket consommé, à vérifier par l'appelant. Titulaire : un admin, ou un propriétaire
/// (ADR-0054), jamais les deux.
#[derive(Debug, Clone)]
pub struct RedeemedTicket {
    pub server_id: Uuid,
    pub backup_id: Option<Uuid>,
    pub admin_id: Option<Uuid>,
    pub owner_ref: Option<String>,
    pub expired: bool,
}

#[derive(Debug)]
pub struct TicketRepository;

impl TicketRepository {
    /// Enregistre un ticket ; les tickets expirés sont purgés au passage.
    pub async fn issue(
        conn: &mut PgConnection,
        token_hash: &str,
        server_id: Uuid,
        backup_id: Option<Uuid>,
        admin_id: Option<Uuid>,
        owner_ref: Option<&str>,
        ttl: Duration,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!("DELETE FROM tickets WHERE expires_at < now()")
            .execute(&mut *conn)
            .await?;
        sqlx::query!(
            "INSERT INTO tickets (token_hash, server_id, backup_id, admin_id, owner_ref, expires_at)
             VALUES ($1, $2, $3, $4, $5, now() + $6 * interval '1 second')",
            token_hash,
            server_id,
            backup_id,
            admin_id,
            owner_ref,
            ttl.as_secs_f64(),
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    /// Consomme le ticket, qu'il soit valide ou non : un ticket ne se présente qu'une fois.
    pub async fn redeem(
        conn: &mut PgConnection,
        token_hash: &str,
    ) -> Result<Option<RedeemedTicket>, sqlx::Error> {
        let row = sqlx::query!(
            r#"DELETE FROM tickets WHERE token_hash = $1
               RETURNING server_id, backup_id, admin_id, owner_ref,
                         (expires_at <= now()) AS "expired!""#,
            token_hash
        )
        .fetch_optional(&mut *conn)
        .await?;
        Ok(row.map(|row| RedeemedTicket {
            server_id: row.server_id,
            backup_id: row.backup_id,
            admin_id: row.admin_id,
            owner_ref: row.owner_ref,
            expired: row.expired,
        }))
    }
}
