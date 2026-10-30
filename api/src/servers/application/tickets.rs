//! Tickets à usage unique, courts, obtenus par une requête REST authentifiée puis présentés sur
//! le WebSocket de console (ADR-0014) ou au téléchargement d'une sauvegarde (ADR-0018). En base :
//! délivrés par une instance, consommables par une autre (ADR-0037).

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use super::ServersError;
use crate::auth::opaque;
use crate::servers::infra::TicketRepository;

/// Durée de validité d'un ticket.
pub const TICKET_TTL: Duration = Duration::from_secs(30);
const TICKET_PREFIX: &str = "kct_";

pub async fn issue_console(
    pool: &PgPool,
    server_id: Uuid,
    admin_id: Uuid,
) -> Result<String, ServersError> {
    issue(pool, server_id, None, admin_id).await
}

/// L'admin qui a obtenu le ticket, s'il est valide et pour ce serveur. Un ticket présenté pour
/// un autre serveur est tout de même consommé.
pub async fn redeem_console(
    pool: &PgPool,
    ticket: &str,
    server_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    redeem(pool, ticket, server_id, None).await
}

pub async fn issue_download(
    pool: &PgPool,
    server_id: Uuid,
    backup_id: Uuid,
    admin_id: Uuid,
) -> Result<String, ServersError> {
    issue(pool, server_id, Some(backup_id), admin_id).await
}

pub async fn redeem_download(
    pool: &PgPool,
    ticket: &str,
    server_id: Uuid,
    backup_id: Uuid,
) -> Result<Option<Uuid>, sqlx::Error> {
    redeem(pool, ticket, server_id, Some(backup_id)).await
}

async fn issue(
    pool: &PgPool,
    server_id: Uuid,
    backup_id: Option<Uuid>,
    admin_id: Uuid,
) -> Result<String, ServersError> {
    let ticket =
        opaque::generate(TICKET_PREFIX).map_err(|e| ServersError::Internal(e.to_string()))?;
    let mut conn = pool.acquire().await?;
    TicketRepository::issue(
        &mut conn,
        &opaque::hash(&ticket),
        server_id,
        backup_id,
        admin_id,
        TICKET_TTL,
    )
    .await?;
    Ok(ticket)
}

async fn redeem(
    pool: &PgPool,
    ticket: &str,
    server_id: Uuid,
    backup_id: Option<Uuid>,
) -> Result<Option<Uuid>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let Some(stored) = TicketRepository::redeem(&mut conn, &opaque::hash(ticket)).await? else {
        return Ok(None);
    };
    Ok(
        (stored.server_id == server_id && stored.backup_id == backup_id && !stored.expired)
            .then_some(stored.admin_id),
    )
}
