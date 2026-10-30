use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit::domain::chain::{self, ChainRecord, GENESIS};
use crate::audit::domain::{AuditContext, AuditEvent};

/// Clé du verrou consultatif qui sérialise les insertions (chaînage).
const CHAIN_LOCK: i64 = 0x6b75_726f_6175_6474;

/// Ligne du journal telle que stockée.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub seq: i64,
    pub id: Uuid,
    pub occurred_at: OffsetDateTime,
    pub actor_kind: String,
    pub actor_id: Option<Uuid>,
    pub action: String,
    pub outcome: String,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Option<String>,
    pub details: Value,
    pub prev_hash: String,
    pub hash: String,
}

impl AuditRecord {
    pub fn chain_record(&self) -> ChainRecord<'_> {
        ChainRecord {
            id: self.id,
            occurred_at: self.occurred_at,
            actor_kind: &self.actor_kind,
            actor_id: self.actor_id,
            action: &self.action,
            outcome: &self.outcome,
            target_kind: self.target_kind.as_deref(),
            target_id: self.target_id.as_deref(),
            ip: self.ip.as_deref(),
            user_agent: self.user_agent.as_deref(),
            request_id: self.request_id.as_deref(),
            details: &self.details,
        }
    }
}

/// Critères de consultation, du plus récent au plus ancien.
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// Pagination : événements de numéro strictement inférieur.
    pub before_seq: Option<i64>,
    pub actor_id: Option<Uuid>,
    /// Préfixe de code d'action, par exemple `iam.` ou `auth.login`.
    pub action_prefix: Option<String>,
    /// `admin`, `system`, `node`, `cli`…
    pub actor_kind: Option<String>,
    /// `success`, `denied`, `failure`.
    pub outcome: Option<String>,
    /// Période : depuis (inclus), jusqu'à (exclu).
    pub from: Option<OffsetDateTime>,
    pub to: Option<OffsetDateTime>,
    pub limit: i64,
}

#[derive(Debug)]
pub struct AuditRepository;

impl AuditRepository {
    /// Ajoute un événement en fin de chaîne. Réservé à [`super::AuditedTx`].
    pub(super) async fn append(
        conn: &mut PgConnection,
        context: &AuditContext,
        event: &AuditEvent,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(CHAIN_LOCK)
            .execute(&mut *conn)
            .await?;
        let prev_hash = sqlx::query_scalar!("SELECT hash FROM audit_log ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *conn)
            .await?
            .unwrap_or_else(|| GENESIS.to_owned());

        // Agent IA : son client et son accès rejoignent le détail, couvert par le chaînage.
        let details = match &context.agent {
            Some(agent) => {
                let mut details = match &event.details {
                    serde_json::Value::Null => serde_json::json!({}),
                    other => other.clone(),
                };
                if let Some(object) = details.as_object_mut() {
                    object.insert(
                        "agent".to_owned(),
                        serde_json::json!({ "client": agent.client_id, "grant": agent.grant_id }),
                    );
                }
                details
            }
            None => event.details.clone(),
        };
        let record = ChainRecord {
            id: Uuid::now_v7(),
            occurred_at: chain::now(),
            actor_kind: context.actor.kind.as_str(),
            actor_id: context.actor.id,
            action: event.action.code(),
            outcome: event.outcome.as_str(),
            target_kind: event.target.as_ref().map(|target| target.kind),
            target_id: event.target.as_ref().map(|target| target.id.as_str()),
            ip: context.ip.as_deref(),
            user_agent: context.user_agent.as_deref(),
            request_id: context.request_id.as_deref(),
            details: &details,
        };
        let hash = record.hash(&prev_hash);

        sqlx::query!(
            "INSERT INTO audit_log (id, occurred_at, actor_kind, actor_id, action, outcome,
                target_kind, target_id, ip, user_agent, request_id, details, prev_hash, hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            record.id,
            record.occurred_at,
            record.actor_kind,
            record.actor_id,
            record.action,
            record.outcome,
            record.target_kind,
            record.target_id,
            record.ip,
            record.user_agent,
            record.request_id,
            record.details,
            prev_hash,
            hash,
        )
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    pub async fn list(
        conn: &mut PgConnection,
        filter: &AuditFilter,
    ) -> Result<Vec<AuditRecord>, sqlx::Error> {
        sqlx::query_as!(
            AuditRecord,
            "SELECT seq, id, occurred_at, actor_kind, actor_id, action, outcome, target_kind,
                    target_id, ip, user_agent, request_id, details, prev_hash, hash
             FROM audit_log
             WHERE ($1::bigint IS NULL OR seq < $1)
               AND ($2::uuid IS NULL OR actor_id = $2)
               AND ($3::text IS NULL OR starts_with(action, $3))
               AND ($4::text IS NULL OR actor_kind = $4)
               AND ($5::text IS NULL OR outcome = $5)
               AND ($6::timestamptz IS NULL OR occurred_at >= $6)
               AND ($7::timestamptz IS NULL OR occurred_at < $7)
             ORDER BY seq DESC
             LIMIT $8",
            filter.before_seq,
            filter.actor_id,
            filter.action_prefix,
            filter.actor_kind,
            filter.outcome,
            filter.from,
            filter.to,
            filter.limit,
        )
        .fetch_all(conn)
        .await
    }

    /// Événements filtrés de numéro strictement supérieur, du plus ancien au plus récent
    /// (export). `before_seq` et `limit` du filtre sont ignorés.
    pub async fn export_page(
        pool: &PgPool,
        filter: &AuditFilter,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<AuditRecord>, sqlx::Error> {
        sqlx::query_as!(
            AuditRecord,
            "SELECT seq, id, occurred_at, actor_kind, actor_id, action, outcome, target_kind,
                    target_id, ip, user_agent, request_id, details, prev_hash, hash
             FROM audit_log
             WHERE seq > $1
               AND ($2::uuid IS NULL OR actor_id = $2)
               AND ($3::text IS NULL OR starts_with(action, $3))
               AND ($4::text IS NULL OR actor_kind = $4)
               AND ($5::text IS NULL OR outcome = $5)
               AND ($6::timestamptz IS NULL OR occurred_at >= $6)
               AND ($7::timestamptz IS NULL OR occurred_at < $7)
             ORDER BY seq
             LIMIT $8",
            after_seq,
            filter.actor_id,
            filter.action_prefix,
            filter.actor_kind,
            filter.outcome,
            filter.from,
            filter.to,
            limit,
        )
        .fetch_all(pool)
        .await
    }

    /// Lignes de numéro strictement supérieur, dans l'ordre de la chaîne.
    pub async fn after(
        pool: &PgPool,
        after_seq: i64,
        limit: i64,
    ) -> Result<Vec<AuditRecord>, sqlx::Error> {
        sqlx::query_as!(
            AuditRecord,
            "SELECT seq, id, occurred_at, actor_kind, actor_id, action, outcome, target_kind,
                    target_id, ip, user_agent, request_id, details, prev_hash, hash
             FROM audit_log WHERE seq > $1 ORDER BY seq LIMIT $2",
            after_seq,
            limit,
        )
        .fetch_all(pool)
        .await
    }

    /// Supprime les événements antérieurs à `cutoff` (seule suppression autorisée par le trigger).
    /// Supprime les événements antérieurs à `cutoff` par lots de `batch` lignes, chacun dans
    /// sa transaction : des millions de lignes ne tiennent ni un verrou long ni un journal de
    /// transactions massif. Retourne le nombre de lignes supprimées.
    pub async fn purge_before(
        pool: &PgPool,
        cutoff: OffsetDateTime,
        batch: i64,
    ) -> Result<u64, sqlx::Error> {
        let mut total = 0;
        loop {
            let mut tx = pool.begin().await?;
            sqlx::query("SET LOCAL kuro.audit_purge = 'on'")
                .execute(&mut *tx)
                .await?;
            let deleted = sqlx::query!(
                "DELETE FROM audit_log
                 WHERE seq IN (SELECT seq FROM audit_log WHERE occurred_at < $1
                               ORDER BY seq LIMIT $2)",
                cutoff,
                batch
            )
            .execute(&mut *tx)
            .await?
            .rows_affected();
            tx.commit().await?;
            total += deleted;
            if i64::try_from(deleted).unwrap_or(i64::MAX) < batch {
                return Ok(total);
            }
        }
    }
}
