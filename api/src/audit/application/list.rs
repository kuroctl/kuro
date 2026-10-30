use serde_json::json;
use sqlx::PgPool;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::{AuditFilter, AuditRecord, AuditRepository, AuditedTx};

/// Nombre maximal d'événements par page.
pub const MAX_PAGE: i64 = 200;

/// Consultation du journal, elle-même journalisée (ISO 27001 : accès aux journaux tracé).
pub async fn list(
    pool: &PgPool,
    context: &AuditContext,
    mut filter: AuditFilter,
) -> Result<Vec<AuditRecord>, sqlx::Error> {
    filter.limit = filter.limit.clamp(1, MAX_PAGE);
    let mut tx = AuditedTx::begin(pool, context).await?;
    let records = AuditRepository::list(&mut tx, &filter).await?;
    tx.commit(AuditEvent::success(AuditAction::AuditRead).details(json!({
        "before_seq": filter.before_seq,
        "actor_id": filter.actor_id,
        "action_prefix": filter.action_prefix,
        "actor_kind": filter.actor_kind,
        "outcome": filter.outcome,
        "returned": records.len(),
    })))
    .await?;
    Ok(records)
}
