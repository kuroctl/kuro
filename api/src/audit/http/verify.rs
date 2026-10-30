use axum::extract::State;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

use super::list::AuditQuery;
use crate::audit::application;
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Query};
use crate::iam::application::{IamError, IamState};
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(Debug, Serialize, ToSchema)]
pub struct AuditVerifyDto {
    /// Aucune rupture : la chaîne de hash est intacte.
    pub intact: bool,
    pub checked: u64,
    pub first_seq: Option<i64>,
    pub last_seq: Option<i64>,
    /// Premier événement dont le hash ou le lien avec le précédent ne correspond pas.
    pub broken_at: Option<i64>,
}

/// Recalcule la chaîne de hash du journal. La vérification est journalisée.
/// Permission : `audit.read`.
#[utoipa::path(post, path = "/api/admin/v1/audit/verify", tag = "admin-audit", security(("bearer" = [])),
    responses((status = 200, body = AuditVerifyDto), (status = 403, body = Problem)))]
pub async fn verify(
    State(state): State<IamState>,
    session: AdminSession,
) -> Result<Json<AuditVerifyDto>, ApiError> {
    session.require(&state, Permission::AuditRead).await?;
    let report = application::verify_audited(&state.pool, &session.context)
        .await
        .map_err(IamError::from)?;
    Ok(Json(AuditVerifyDto {
        intact: report.is_intact(),
        checked: report.checked,
        first_seq: report.first_seq,
        last_seq: report.last_seq,
        broken_at: report.broken_at,
    }))
}

/// Export CSV des événements filtrés (mêmes filtres que la liste, sans pagination), du plus
/// ancien au plus récent. L'export est journalisé. Permission : `audit.read`.
#[utoipa::path(get, path = "/api/admin/v1/audit/export", tag = "admin-audit", security(("bearer" = [])),
    params(AuditQuery),
    responses((status = 200, content_type = "text/csv", body = String), (status = 403, body = Problem)))]
pub async fn export(
    State(state): State<IamState>,
    session: AdminSession,
    Query(query): Query<AuditQuery>,
) -> Result<Response, ApiError> {
    session.require(&state, Permission::AuditRead).await?;
    let body = application::export(&state.pool, &session.context, query.filter())
        .await
        .map_err(IamError::from)?;
    let date = time::OffsetDateTime::now_utc().date();
    Ok((
        [
            (CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"kuro-audit-{date}.csv\""),
            ),
        ],
        axum::body::Body::from_stream(body),
    )
        .into_response())
}
