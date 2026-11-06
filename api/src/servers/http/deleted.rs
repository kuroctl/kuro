//! `/servers/deleted` : serveurs supprimés dont le nœud garde encore les sauvegardes (ADR-0059).

use axum::extract::State;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::{ServersState, deleted_backups};
use crate::servers::infra::DeletedBackupRecord;

#[derive(Debug, Serialize, ToSchema)]
pub struct KeptBackupDto {
    pub id: Uuid,
    pub name: String,
    pub size_bytes: Option<i64>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeletedServerDto {
    /// Identifiant du serveur supprimé.
    pub id: Uuid,
    pub name: String,
    pub prefab_id: String,
    pub prefab_version: String,
    pub owner_ref: Option<String>,
    pub node_id: Uuid,
    pub node_name: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub deleted_at: OffsetDateTime,
    /// Effacement des sauvegardes par le nœud.
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub purge_at: OffsetDateTime,
    pub backups: Vec<KeptBackupDto>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AdoptRequest {
    /// Serveur du même nœud qui reprend les sauvegardes.
    pub server_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AdoptedDto {
    pub adopted: u64,
}

/// Une entrée par serveur supprimé, ses sauvegardes groupées (la liste arrive triée par serveur).
fn grouped(records: Vec<DeletedBackupRecord>) -> Vec<DeletedServerDto> {
    let mut servers: Vec<DeletedServerDto> = Vec::new();
    for r in records {
        let backup = KeptBackupDto {
            id: r.id,
            name: r.name,
            size_bytes: r.size_bytes,
            created_at: r.created_at,
        };
        match servers.last_mut() {
            Some(server) if server.id == r.server_id => server.backups.push(backup),
            _ => servers.push(DeletedServerDto {
                id: r.server_id,
                name: r.server_name,
                prefab_id: r.prefab_id,
                prefab_version: r.prefab_version,
                owner_ref: r.owner_ref,
                node_id: r.node_id,
                node_name: r.node_name,
                deleted_at: r.deleted_at,
                purge_at: r.purge_at,
                backups: vec![backup],
            }),
        }
    }
    servers
}

/// Serveurs supprimés dont le nœud garde encore les sauvegardes, du plus récent au plus ancien.
/// Permission : `servers.backups.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/deleted", tag = "admin-servers", security(("bearer" = [])),
    responses((status = 200, body = [DeletedServerDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
) -> Result<Json<Vec<DeletedServerDto>>, ApiError> {
    session
        .require(&iam, Permission::ServersBackupsRead)
        .await?;
    Ok(Json(grouped(deleted_backups::list(&state).await?)))
}

/// Un serveur du même nœud reprend les sauvegardes gardées ; elles deviennent les siennes, à
/// restaurer ensuite. Permission : `servers.backups.restore`.
#[utoipa::path(post, path = "/api/admin/v1/servers/deleted/{id}/adopt", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Serveur supprimé")),
    request_body = AdoptRequest,
    responses((status = 200, body = AdoptedDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Autre nœud, serveur occupé ou nœud hors ligne", body = Problem),
        (status = 502, body = Problem)))]
pub async fn adopt(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<AdoptRequest>,
) -> Result<Json<AdoptedDto>, ApiError> {
    session
        .require(&iam, Permission::ServersBackupsRestore)
        .await?;
    let adopted = deleted_backups::adopt(&state, &session.context, id, request.server_id).await?;
    Ok(Json(AdoptedDto { adopted }))
}
