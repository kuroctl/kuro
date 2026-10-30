use sqlx::PgPool;

use crate::audit::domain::{AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;

/// Événement sans modification associée (refus d'accès, échec d'authentification…).
pub async fn record(
    pool: &PgPool,
    context: &AuditContext,
    event: AuditEvent,
) -> Result<(), sqlx::Error> {
    AuditedTx::begin(pool, context).await?.commit(event).await
}
