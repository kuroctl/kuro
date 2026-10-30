use serde_json::json;
use sqlx::PgPool;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::{AuditRepository, AuditedTx};

const BATCH: i64 = 1_000;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub checked: u64,
    pub first_seq: Option<i64>,
    pub last_seq: Option<i64>,
    /// Première ligne dont le hash ou le lien avec la précédente ne correspond pas.
    pub broken_at: Option<i64>,
}

impl VerifyReport {
    pub fn is_intact(&self) -> bool {
        self.broken_at.is_none()
    }
}

/// Vérification demandée depuis le panel, journalisée avec son résultat.
pub async fn verify_audited(
    pool: &PgPool,
    context: &AuditContext,
) -> Result<VerifyReport, sqlx::Error> {
    let report = verify(pool).await?;
    let tx = AuditedTx::begin(pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::AuditVerified).details(json!({
            "checked": report.checked,
            "broken_at": report.broken_at,
        })),
    )
    .await?;
    Ok(report)
}

/// Recalcule toute la chaîne. La première ligne restante (après une purge) sert d'ancre :
/// seul son propre hash est vérifié.
pub async fn verify(pool: &PgPool) -> Result<VerifyReport, sqlx::Error> {
    let mut report = VerifyReport::default();
    let mut previous_hash: Option<String> = None;
    let mut cursor = 0;

    loop {
        let batch = AuditRepository::after(pool, cursor, BATCH).await?;
        let Some(last) = batch.last() else {
            return Ok(report);
        };
        cursor = last.seq;

        for record in &batch {
            report.first_seq.get_or_insert(record.seq);
            report.last_seq = Some(record.seq);
            report.checked += 1;

            let linked = previous_hash
                .as_deref()
                .is_none_or(|previous| previous == record.prev_hash);
            if !linked || record.chain_record().hash(&record.prev_hash) != record.hash {
                report.broken_at = Some(record.seq);
                return Ok(report);
            }
            previous_hash = Some(record.hash.clone());
        }
    }
}
