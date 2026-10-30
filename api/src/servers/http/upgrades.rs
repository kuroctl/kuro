//! Mise à jour vers une version plus récente du prefab (ADR-0024). Permission :
//! `servers.upgrade` ; la sauvegarde préalable exige aussi `servers.backups.create`.

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::upgrade::{self, Blocker, Preview};
use crate::servers::application::{self as servers, ServersState};

#[derive(Debug, Serialize, ToSchema)]
pub struct RenamedSettingDto {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImageChangeDto {
    pub service: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// Ce qui empêche la mise à jour.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpgradeBlockerDto {
    /// Valeur devenue invalide (`reason` : `options`, `max`, `required`…).
    Setting { field: String, reason: String },
    /// Emplacement de contenu retiré alors qu'il contient des mods.
    ContentSlot { slot: String },
    /// Pas assez de ports libres sur le nœud.
    NoFreePort { needed: usize, free: i64 },
    /// Version marquée brouillon, non déployable.
    NotDeployable,
    /// Ressource (`memory_mb`, `cpu_percent`, `disk_mb`) sous le minimum de la nouvelle version.
    ResourceBelowMinimum {
        resource: String,
        minimum: u32,
        current: i32,
    },
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpgradePreviewDto {
    pub from: String,
    pub to: String,
    pub renamed: Vec<RenamedSettingDto>,
    pub removed: Vec<String>,
    /// Nouveaux réglages, à leur valeur par défaut.
    pub added: Vec<String>,
    pub ports_added: Vec<String>,
    pub ports_removed: Vec<String>,
    pub images: Vec<ImageChangeDto>,
    /// Étapes d'installation relancées, données gardées.
    pub reinstall: bool,
    pub blockers: Vec<UpgradeBlockerDto>,
}

impl From<Preview> for UpgradePreviewDto {
    fn from(p: Preview) -> Self {
        Self {
            from: p.from,
            to: p.to,
            renamed: p
                .renamed
                .into_iter()
                .map(|(from, to)| RenamedSettingDto { from, to })
                .collect(),
            removed: p.removed,
            added: p.added,
            ports_added: p.ports_added,
            ports_removed: p.ports_removed,
            images: p
                .images
                .into_iter()
                .map(|i| ImageChangeDto {
                    service: i.service,
                    from: i.from,
                    to: i.to,
                })
                .collect(),
            reinstall: p.reinstall,
            blockers: p
                .blockers
                .into_iter()
                .map(|b| match b {
                    Blocker::Setting { field, reason } => UpgradeBlockerDto::Setting {
                        field,
                        reason: reason.to_owned(),
                    },
                    Blocker::ContentSlot { slot } => UpgradeBlockerDto::ContentSlot { slot },
                    Blocker::NoFreePort { needed, free } => {
                        UpgradeBlockerDto::NoFreePort { needed, free }
                    }
                    Blocker::NotDeployable => UpgradeBlockerDto::NotDeployable,
                    Blocker::ResourceBelowMinimum {
                        resource,
                        minimum,
                        current,
                    } => UpgradeBlockerDto::ResourceBelowMinimum {
                        resource: resource.to_owned(),
                        minimum,
                        current,
                    },
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpgradesDto {
    pub current: String,
    /// Versions plus récentes, de la plus ancienne à la plus récente.
    pub available: Vec<String>,
}

/// Versions plus récentes du prefab du serveur. Permission : `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/upgrades", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = UpgradesDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn available(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<UpgradesDto>, ApiError> {
    session.require(&iam, Permission::ServersRead).await?;
    let server = servers::get(&state, id).await?.server;
    let available = upgrade::newer_versions(&state, &server).await?;
    Ok(Json(UpgradesDto {
        current: server.prefab_version,
        available,
    }))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct PreviewQuery {
    /// Version visée.
    pub to: String,
}

/// Aperçu de la mise à jour, sans rien écrire. Permission : `servers.upgrade`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}/upgrade", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path), PreviewQuery),
    responses((status = 200, body = UpgradePreviewDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Pas une version plus récente du même prefab (`upgrade_not_newer`)", body = Problem)))]
pub async fn preview(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Query(query): Query<PreviewQuery>,
) -> Result<Json<UpgradePreviewDto>, ApiError> {
    session.require(&iam, Permission::ServersUpgrade).await?;
    Ok(Json(upgrade::preview(&state, id, &query.to).await?.into()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpgradeRequest {
    pub to: String,
    /// Sauvegarde attendue avant la mise à jour.
    #[serde(default)]
    pub backup: bool,
}

async fn require_upgrade(
    iam: &IamState,
    session: &AdminSession,
    backup: bool,
) -> Result<(), ApiError> {
    session.require(iam, Permission::ServersUpgrade).await?;
    if backup {
        session
            .require(iam, Permission::ServersBackupsCreate)
            .await?;
    }
    Ok(())
}

/// Lance la mise à jour : sauvegarde attendue si demandée, réglages migrés, redéploiement.
/// Le résultat suit dans l'état du serveur et le journal (`server.upgraded`).
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/upgrade", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpgradeRequest,
    responses((status = 202, body = UpgradePreviewDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Bloquée, serveur occupé ou suspendu, nœud hors ligne, sauvegardes désactivées", body = Problem),
        (status = 422, body = Problem)))]
pub async fn start(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpgradeRequest>,
) -> Result<(StatusCode, Json<UpgradePreviewDto>), ApiError> {
    require_upgrade(&iam, &session, request.backup).await?;
    let preview = upgrade::start(&state, &session.context, id, &request.to, request.backup).await?;
    Ok((StatusCode::ACCEPTED, Json(preview.into())))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpgradeCandidateDto {
    pub server_id: Uuid,
    pub name: String,
    pub version: String,
    pub state: String,
    pub preview: UpgradePreviewDto,
}

/// Serveurs de ce prefab sur une version plus ancienne, avec leur aperçu. Permission :
/// `servers.upgrade`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}/upgrades", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path)),
    responses((status = 200, body = [UpgradeCandidateDto]), (status = 403, body = Problem)))]
pub async fn candidates(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path((prefab, version)): Path<(String, String)>,
) -> Result<Json<Vec<UpgradeCandidateDto>>, ApiError> {
    session.require(&iam, Permission::ServersUpgrade).await?;
    let candidates = upgrade::candidates(&state, &prefab, &version).await?;
    Ok(Json(
        candidates
            .into_iter()
            .map(|(server, preview)| UpgradeCandidateDto {
                server_id: server.id,
                name: server.name,
                version: server.prefab_version,
                state: server.state,
                preview: preview.into(),
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkUpgradeRequest {
    pub servers: Vec<Uuid>,
    pub to: String,
    #[serde(default)]
    pub backup: bool,
}

/// Issue du lancement pour un serveur ; `error` : code stable de la raison du refus.
#[derive(Debug, Serialize, ToSchema)]
pub struct BulkUpgradeResultDto {
    pub server_id: Uuid,
    pub started: bool,
    pub error: Option<String>,
}

/// Lance la mise à jour de plusieurs serveurs, chacun indépendamment : un refus n'arrête pas
/// les autres.
#[utoipa::path(post, path = "/api/admin/v1/servers/upgrade", tag = "admin-servers", security(("bearer" = [])),
    request_body = BulkUpgradeRequest,
    responses((status = 202, body = [BulkUpgradeResultDto]), (status = 403, body = Problem)))]
pub async fn start_many(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Json(request): Json<BulkUpgradeRequest>,
) -> Result<(StatusCode, Json<Vec<BulkUpgradeResultDto>>), ApiError> {
    require_upgrade(&iam, &session, request.backup).await?;
    let mut results = Vec::with_capacity(request.servers.len());
    for id in request.servers {
        let outcome =
            upgrade::start(&state, &session.context, id, &request.to, request.backup).await;
        results.push(BulkUpgradeResultDto {
            server_id: id,
            started: outcome.is_ok(),
            error: outcome.err().map(|error| error_code(&error)),
        });
    }
    Ok((StatusCode::ACCEPTED, Json(results)))
}

fn error_code(error: &servers::ServersError) -> String {
    use servers::ServersError as E;
    match error {
        E::NotFound => "not_found".to_owned(),
        E::Missing(code) | E::Unprocessable(code) | E::Conflict(code) => code.as_str().to_owned(),
        E::Daemon { code, .. } => code.as_str().to_owned(),
        E::Invalid(field) => field.code.as_str().to_owned(),
        E::InvalidSettings(_) => "invalid_setting".to_owned(),
        E::Forbidden(_) => "forbidden".to_owned(),
        E::Internal(_) => "internal_error".to_owned(),
    }
}
