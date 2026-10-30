use crate::http::extract::Json;
use crate::http::extract::Query;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::audit::application;
use crate::audit::infra::{AuditFilter, AuditRecord};
use crate::http::error::{ApiError, Problem};
use crate::iam::application::{IamError, IamState};
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

const DEFAULT_PAGE: i64 = 50;

#[derive(Debug, Deserialize, IntoParams)]
pub struct AuditQuery {
    /// Pagination : événements antérieurs à ce numéro (`next_before_seq` de la page précédente).
    pub before_seq: Option<i64>,
    pub actor_id: Option<Uuid>,
    /// Préfixe de code d'action, par exemple `iam.` ou `auth.login`.
    pub action: Option<String>,
    /// Type d'acteur : `admin`, `system`, `node`, `cli`.
    pub actor_kind: Option<String>,
    /// `success`, `denied` ou `failure`.
    pub outcome: Option<String>,
    /// Depuis (inclus), RFC 3339.
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[param(value_type = Option<String>, format = DateTime)]
    pub from: Option<OffsetDateTime>,
    /// Jusqu'à (exclu), RFC 3339.
    #[serde(default, with = "time::serde::rfc3339::option")]
    #[param(value_type = Option<String>, format = DateTime)]
    pub to: Option<OffsetDateTime>,
    /// 1 à 200, 50 par défaut.
    pub limit: Option<i64>,
}

impl AuditQuery {
    pub(super) fn filter(self) -> AuditFilter {
        AuditFilter {
            before_seq: self.before_seq,
            actor_id: self.actor_id,
            action_prefix: self.action,
            actor_kind: self.actor_kind,
            outcome: self.outcome,
            from: self.from,
            to: self.to,
            limit: self.limit.unwrap_or(DEFAULT_PAGE),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditEntryDto {
    pub seq: i64,
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub occurred_at: OffsetDateTime,
    #[schema(example = "admin")]
    pub actor_kind: String,
    pub actor_id: Option<Uuid>,
    #[schema(example = "iam.role.created")]
    pub action: String,
    #[schema(example = "success")]
    pub outcome: String,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Option<String>,
    #[schema(value_type = Object)]
    pub details: Value,
    /// Maillon de la chaîne d'intégrité.
    pub hash: String,
}

impl From<AuditRecord> for AuditEntryDto {
    fn from(record: AuditRecord) -> Self {
        Self {
            seq: record.seq,
            id: record.id,
            occurred_at: record.occurred_at,
            actor_kind: record.actor_kind,
            actor_id: record.actor_id,
            action: record.action,
            outcome: record.outcome,
            target_kind: record.target_kind,
            target_id: record.target_id,
            ip: record.ip,
            user_agent: record.user_agent,
            request_id: record.request_id,
            details: record.details,
            hash: record.hash,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditPage {
    pub items: Vec<AuditEntryDto>,
    /// À repasser en `before_seq` pour la page suivante ; absent en fin de journal.
    pub next_before_seq: Option<i64>,
}

/// Journal d'audit, du plus récent au plus ancien. Permission : `audit.read`.
/// La consultation est elle-même journalisée.
#[utoipa::path(get, path = "/api/admin/v1/audit", tag = "admin-audit", security(("bearer" = [])),
    params(AuditQuery),
    responses((status = 200, body = AuditPage), (status = 403, body = Problem)))]
pub async fn list(
    State(state): State<IamState>,
    session: AdminSession,
    Query(query): Query<AuditQuery>,
) -> Result<Json<AuditPage>, ApiError> {
    session.require(&state, Permission::AuditRead).await?;
    let filter = query.filter();
    let limit = filter.limit.clamp(1, application::MAX_PAGE);
    let records = application::list(&state.pool, &session.context, filter)
        .await
        .map_err(IamError::from)?;

    let full_page = i64::try_from(records.len()).is_ok_and(|count| count == limit);
    let next_before_seq = records
        .last()
        .filter(|_| full_page)
        .map(|record| record.seq);
    Ok(Json(AuditPage {
        items: records.into_iter().map(Into::into).collect(),
        next_before_seq,
    }))
}
