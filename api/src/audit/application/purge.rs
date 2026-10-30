use serde_json::json;
use sqlx::PgPool;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent, chain};
use crate::audit::infra::{AuditRepository, AuditedTx};
use crate::settings::infra::SettingsRepository;

/// Délai entre deux purges automatiques.
const PURGE_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// Lignes supprimées par transaction.
const PURGE_BATCH: i64 = 10_000;
/// Première purge peu après le démarrage, une fois l'API prête.
const FIRST_PURGE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// Applique la durée de conservation des paramètres du projet (ADR-0019).
pub async fn purge_expired(
    pool: &PgPool,
    context: &AuditContext,
) -> Result<(u32, u64), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    let retention = SettingsRepository::get(&mut conn)
        .await?
        .audit_retention_days;
    drop(conn);
    let retention = u32::try_from(retention).unwrap_or(u32::MAX);
    Ok((retention, purge(pool, context, retention).await?))
}

/// Purge quotidienne, faite par l'API elle-même ; s'arrête avec le jeton.
pub async fn purge_daily(pool: PgPool, shutdown: tokio_util::sync::CancellationToken) {
    let mut delay = FIRST_PURGE_AFTER;
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        match purge_expired(&pool, &AuditContext::system()).await {
            Ok((retention, deleted)) => {
                tracing::info!(retention, deleted, "audit log purged");
            }
            Err(error) => tracing::error!(%error, "audit purge failed"),
        }
        delay = PURGE_EVERY;
    }
}

/// Applique la durée de conservation (RGPD art. 5.1.e). La purge est elle-même journalisée.
pub async fn purge(
    pool: &PgPool,
    context: &AuditContext,
    retention_days: u32,
) -> Result<u64, sqlx::Error> {
    let cutoff = chain::now() - time::Duration::days(i64::from(retention_days));
    // Par lots, chacun validé : la purge ne tient jamais la table longtemps. L'événement vient
    // ensuite ; interrompue entre deux lots, la purge suivante reprend le reste.
    let deleted = AuditRepository::purge_before(pool, cutoff, PURGE_BATCH).await?;
    let tx = AuditedTx::begin(pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::AuditPurged).details(json!({
            "retention_days": retention_days,
            "deleted": deleted,
        })),
    )
    .await?;
    Ok(deleted)
}
