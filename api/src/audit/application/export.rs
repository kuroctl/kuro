use std::pin::Pin;

use axum::body::Bytes;
use futures_util::{Stream, stream};
use serde_json::json;
use sqlx::PgPool;
use time::format_description::well_known::Rfc3339;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::{AuditFilter, AuditRecord, AuditRepository, AuditedTx};

/// Événements lus par requête : la mémoire de l'API reste bornée quelle que soit la sélection.
const BATCH: i64 = 1_000;

const HEADER: &str = "seq,occurred_at,actor_kind,actor_id,action,outcome,target_kind,target_id,\
ip,user_agent,request_id,details,hash\r\n";

pub type CsvStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// Export CSV des événements filtrés, du plus ancien au plus récent (ADR-0019). L'export est
/// journalisé avec ses filtres avant le premier octet.
pub async fn export(
    pool: &PgPool,
    context: &AuditContext,
    filter: AuditFilter,
) -> Result<CsvStream, sqlx::Error> {
    let tx = AuditedTx::begin(pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::AuditExported).details(json!({
            "actor_id": filter.actor_id,
            "action_prefix": filter.action_prefix,
            "actor_kind": filter.actor_kind,
            "outcome": filter.outcome,
            "from": filter.from.and_then(|t| t.format(&Rfc3339).ok()),
            "to": filter.to.and_then(|t| t.format(&Rfc3339).ok()),
        })),
    )
    .await?;

    let start = (pool.clone(), filter, Some(0_i64), true);
    let body = stream::unfold(start, |(pool, filter, cursor, first)| async move {
        let cursor = cursor?;
        let page = match AuditRepository::export_page(&pool, &filter, cursor, BATCH).await {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(%error, "audit export failed");
                // Connexion coupée : le fichier reste visiblement incomplet.
                return Some((
                    Err(std::io::Error::other(error)),
                    (pool, filter, None, false),
                ));
            }
        };
        let next = (page.len() as i64 == BATCH).then(|| page.last().map_or(cursor, |r| r.seq));
        let mut out = String::new();
        if first {
            out.push_str(HEADER);
        }
        for record in &page {
            row(&mut out, record);
        }
        if out.is_empty() {
            return None;
        }
        Some((Ok(Bytes::from(out)), (pool, filter, next, false)))
    });
    Ok(Box::pin(body))
}

fn row(out: &mut String, r: &AuditRecord) {
    let fields = [
        r.seq.to_string(),
        r.occurred_at.format(&Rfc3339).unwrap_or_default(),
        r.actor_kind.clone(),
        r.actor_id.map(|id| id.to_string()).unwrap_or_default(),
        r.action.clone(),
        r.outcome.clone(),
        r.target_kind.clone().unwrap_or_default(),
        r.target_id.clone().unwrap_or_default(),
        r.ip.clone().unwrap_or_default(),
        r.user_agent.clone().unwrap_or_default(),
        r.request_id.clone().unwrap_or_default(),
        r.details.to_string(),
        r.hash.clone(),
    ];
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        cell(out, field);
    }
    out.push_str("\r\n");
}

/// Cellule CSV (RFC 4180). Une valeur qui commence par `=`, `+`, `-`, `@` (un User-Agent, une
/// cible) est préfixée d'une apostrophe : un tableur ne l'exécute pas comme une formule.
fn cell(out: &mut String, value: &str) {
    let formula = value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let quoted = formula || value.contains([',', '"', '\n', '\r']);
    if quoted {
        out.push('"');
    }
    if formula {
        out.push('\'');
    }
    if quoted {
        out.push_str(&value.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(value);
    }
}
