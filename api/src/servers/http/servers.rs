//! `/servers` : serveurs déployés à partir de prefabs.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use kuro_protocol::api::install::InstallMode;
use kuro_protocol::api::server::PowerAction;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::{self as servers, NewServer, ServerView, ServersState};
use crate::servers::domain::ServerState;

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerNodeDto {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerPrefabDto {
    #[schema(example = "minecraft-java")]
    pub id: String,
    #[schema(example = "1.0.0")]
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerAllocationDto {
    /// Port du prefab servi (`game`, `http`…).
    pub name: String,
    pub ip: String,
    pub port: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerDto {
    pub id: Uuid,
    pub name: String,
    pub node: ServerNodeDto,
    pub prefab: ServerPrefabDto,
    pub memory_mb: i32,
    /// 100 = un cœur.
    pub cpu_percent: i32,
    pub disk_mb: i32,
    /// Sauvegardes gardées ; 0 les désactive.
    pub backup_limit: i32,
    pub state: ServerState,
    /// Cause d'un échec (`failed`), ou d'un arrêt inattendu.
    pub state_reason: Option<String>,
    /// Suspendu par un admin (ADR-0019) : ni démarrage, ni fichiers, ni sauvegarde.
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub suspended_at: Option<OffsetDateTime>,
    /// Espace occupé (volumes et sauvegardes), mesuré par le nœud ; absent avant la première
    /// mesure.
    pub disk_used_bytes: Option<i64>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub disk_measured_at: Option<OffsetDateTime>,
    /// Réglages du prefab, sans les secrets.
    #[schema(value_type = Object)]
    pub settings: Value,
    /// Secrets définis (générés au déploiement) ; leurs valeurs ne sont jamais renvoyées.
    pub secrets: Vec<String>,
    pub allocations: Vec<ServerAllocationDto>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
    /// Serveur d'essai du Studio, supprimé à son expiration.
    pub trial: Option<ServerTrialDto>,
    /// Des changements de contenu attendent le prochain démarrage (le serveur tournait).
    pub restart_required: bool,
    pub pending_changes: PendingChangesDto,
    /// Nom dans la zone DNS (`survie` pour `survie.<domaine>`, ADR-0026) ; attribué à la
    /// première synchronisation.
    pub dns_label: Option<String>,
    /// Propriétaire (ADR-0051) : identifiant opaque posé par une extension (`cloud:…`) ;
    /// absent pour un serveur interne.
    #[schema(example = "cloud:6f1c2a9e-3b7d-4c8e-9a51-2d4f8b0c7e13")]
    pub owner_ref: Option<String>,
    /// Identifiant du site web (`https://<web_id>.<domaine des sites>`, ADR-0038) ; présent pour
    /// un serveur dont le prefab a un port web.
    pub web_id: Option<String>,
    /// État du site rapporté par le nœud ; absent tant que le nœud n'a rien dit.
    pub web_status: Option<ServerWebStatusDto>,
}

/// État du site web d'un serveur (ADR-0038), tel que son nœud le rapporte.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerWebStatusDto {
    pub proxy: WebProxyStateDto,
    pub certificate: WebCertificateStateDto,
    /// Pourquoi le proxy est indisponible.
    pub message: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebProxyStateDto {
    Ready,
    Unavailable,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebCertificateStateDto {
    Pending,
    Issued,
}

fn web_status_dto(status: Option<&Value>) -> Option<ServerWebStatusDto> {
    use kuro_protocol::daemon::server::{CertificateState, ProxyState, WebStatus};
    let status: WebStatus = serde_json::from_value(status?.clone()).ok()?;
    Some(ServerWebStatusDto {
        proxy: match status.proxy {
            ProxyState::Ready => WebProxyStateDto::Ready,
            ProxyState::Unavailable => WebProxyStateDto::Unavailable,
        },
        certificate: match status.certificate {
            CertificateState::Pending => WebCertificateStateDto::Pending,
            CertificateState::Issued => WebCertificateStateDto::Issued,
        },
        message: status.message,
    })
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerTrialDto {
    /// Brouillon essayé ; absent s'il a été supprimé ou publié depuis.
    pub draft_id: Option<Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: OffsetDateTime,
}

/// Changements de contenu en attente du prochain démarrage.
#[derive(Debug, Serialize, ToSchema)]
pub struct PendingChangesDto {
    /// Contenus ajoutés, activés, désactivés ou modifiés.
    pub contents: i64,
    /// Contenus retirés.
    pub removals: i64,
}

pub(super) fn dto(state: &ServersState, view: ServerView) -> ServerDto {
    let secrets = servers::secret_keys(state, &view.server);
    let server = view.server;
    ServerDto {
        id: server.id,
        name: server.name,
        node: ServerNodeDto {
            id: server.node_id,
            name: server.node_name,
        },
        prefab: ServerPrefabDto {
            id: server.prefab_id,
            version: server.prefab_version,
        },
        memory_mb: server.memory_mb,
        cpu_percent: server.cpu_percent,
        disk_mb: server.disk_mb,
        backup_limit: server.backup_limit,
        state: server.state.parse().unwrap_or(ServerState::Failed),
        state_reason: server.state_reason,
        suspended_at: server.suspended_at,
        disk_used_bytes: server.disk_used_bytes,
        disk_measured_at: server.disk_measured_at,
        settings: server.settings,
        secrets,
        allocations: view
            .allocations
            .into_iter()
            .map(|(name, ip, port)| ServerAllocationDto { name, ip, port })
            .collect(),
        created_at: server.created_at,
        updated_at: server.updated_at,
        dns_label: server.dns_label.clone(),
        owner_ref: server.owner_ref.clone(),
        web_id: server.web_id.clone(),
        web_status: web_status_dto(server.web_status.as_ref()),
        trial: server.expires_at.map(|expires_at| ServerTrialDto {
            draft_id: server.trial_draft_id,
            expires_at,
        }),
        restart_required: view.pending.restart_required,
        pending_changes: PendingChangesDto {
            contents: view.pending.contents,
            removals: view.pending.removals,
        },
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServerRequest {
    #[schema(example = "Survie")]
    pub name: String,
    pub node_id: Uuid,
    #[schema(example = "minecraft-java")]
    pub prefab_id: String,
    /// Dernière version importée si absente.
    pub prefab_version: Option<String>,
    /// Valeur par défaut des paramètres du projet si absente, comme les autres ressources.
    #[schema(example = 4096)]
    pub memory_mb: Option<i32>,
    /// 100 = un cœur.
    #[schema(example = 200)]
    pub cpu_percent: Option<i32>,
    /// 0 : illimité, refusé s'il existe un plafond disque.
    #[schema(example = 10240)]
    pub disk_mb: Option<i32>,
    /// Réglages du prefab ; les autres prennent leur valeur par défaut.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub settings: Map<String, Value>,
    /// Port public du prefab → allocation ; sinon la première libre du nœud.
    #[serde(default)]
    pub allocations: BTreeMap<String, Uuid>,
    /// Démarrer le serveur une fois déployé et installé (permission `servers.power` en plus).
    #[serde(default)]
    pub start_when_installed: bool,
    /// Sauvegardes gardées (0 à 100, 0 les désactive).
    #[schema(example = 3)]
    pub backup_limit: Option<i32>,
    /// Propriétaire (ADR-0051) : `<extension>:<identifiant>`, opaque pour Kuro ; absent pour un
    /// serveur interne.
    #[schema(example = "cloud:6f1c2a9e-3b7d-4c8e-9a51-2d4f8b0c7e13")]
    pub owner_ref: Option<String>,
}

/// Nouveau propriétaire ; `null` rend le serveur interne.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetOwnerRequest {
    #[schema(example = "cloud:6f1c2a9e-3b7d-4c8e-9a51-2d4f8b0c7e13")]
    pub owner_ref: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSettingsRequest {
    /// Réglages modifiés ; `null` remet la valeur par défaut.
    #[schema(value_type = Object)]
    pub settings: Map<String, Value>,
}

/// Ressources modifiées seulement (ADR-0019).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateResourcesRequest {
    pub memory_mb: Option<i32>,
    /// 100 = un cœur.
    pub cpu_percent: Option<i32>,
    /// 0 : illimité, refusé s'il existe un plafond disque.
    pub disk_mb: Option<i32>,
    /// Baisser la limite ne supprime rien : la rotation fait place à la prochaine sauvegarde.
    pub backup_limit: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PowerActionDto {
    Start,
    Stop,
    Restart,
    Kill,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PowerRequest {
    pub action: PowerActionDto,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReinstallModeDto {
    /// Étapes du prefab relancées sur les données existantes (mise à jour du jeu).
    Reinstall,
    /// Volumes vidés, puis installation : toutes les données du serveur sont perdues.
    Reset,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReinstallRequest {
    pub mode: ReinstallModeDto,
}

/// Fenêtre de la liste des serveurs ; sans `limit`, tous (le panel passe par `/servers/search`,
/// paginée et filtrée, ADR-0025).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Serveurs rendus au plus (1 à 1000).
    pub limit: Option<usize>,
    /// Serveurs sautés, par nom.
    pub offset: Option<usize>,
    /// Serveurs de ce propriétaire seulement (ADR-0051).
    pub owner_ref: Option<String>,
}

/// Serveurs rendus au plus par une liste bornée.
pub const MAX_LIST_LIMIT: usize = 1000;

/// Serveurs, par nom. Permission : `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers", tag = "admin-servers", security(("bearer" = [])),
    params(ListQuery),
    responses((status = 200, body = [ServerDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<ServerDto>>, ApiError> {
    session.require(&iam, Permission::ServersRead).await?;
    let limit = query.limit.map(|limit| limit.clamp(1, MAX_LIST_LIMIT));
    let views = servers::list(
        &state,
        query.owner_ref.as_deref(),
        limit,
        query.offset.unwrap_or(0),
    )
    .await?;
    Ok(Json(views.into_iter().map(|v| dto(&state, v)).collect()))
}

/// Détail d'un serveur. Permission : `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/{id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ServerDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<ServerDto>, ApiError> {
    session.require(&iam, Permission::ServersRead).await?;
    let view = servers::get(&state, id).await?;
    Ok(Json(dto(&state, view)))
}

/// Déploie un serveur sur un nœud en ligne. Le déploiement continue en arrière-plan : suivre
/// `state` (`deploying` → `installing` → `offline`, ou `failed`). Permission : `servers.create`,
/// et `servers.power` avec `start_when_installed`.
#[utoipa::path(post, path = "/api/admin/v1/servers", tag = "admin-servers", security(("bearer" = [])),
    request_body = CreateServerRequest,
    responses((status = 201, body = ServerDto), (status = 403, body = Problem),
        (status = 409, description = "Nœud hors ligne ou aucun port libre", body = Problem),
        (status = 422, description = "Champ ou réglage invalide, ou ressource au-delà d'un plafond", body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Json(request): Json<CreateServerRequest>,
) -> Result<(StatusCode, Json<ServerDto>), ApiError> {
    session.require(&iam, Permission::ServersCreate).await?;
    if request.start_when_installed {
        session.require(&iam, Permission::ServersPower).await?;
    }
    let view = servers::create(
        &state,
        &session.context,
        NewServer {
            name: request.name,
            node_id: request.node_id,
            prefab_id: request.prefab_id,
            prefab_version: request.prefab_version,
            memory_mb: request.memory_mb,
            cpu_percent: request.cpu_percent,
            disk_mb: request.disk_mb,
            settings: request.settings,
            allocations: request.allocations,
            start_when_installed: request.start_when_installed,
            backup_limit: request.backup_limit,
            owner_ref: request.owner_ref,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(dto(&state, view))))
}

/// Change le propriétaire d'un serveur (transfert, ADR-0051) ; `null` le rend interne. Kuro ne
/// vérifie que le format : l'extension qui a posé l'identifiant en répond. Permission :
/// `servers.update`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/owner", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = SetOwnerRequest,
    responses((status = 200, body = ServerDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Identifiant mal formé, ou propriétaire inchangé", body = Problem)))]
pub async fn set_owner(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<SetOwnerRequest>,
) -> Result<Json<ServerDto>, ApiError> {
    session.require(&iam, Permission::ServersUpdate).await?;
    let view = servers::set_owner(&state, &session.context, id, request.owner_ref).await?;
    Ok(Json(dto(&state, view)))
}

/// Modifie des réglages et redéploie le serveur (redémarré s'il tournait).
/// Permission : `servers.update`.
#[utoipa::path(patch, path = "/api/admin/v1/servers/{id}/settings", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateSettingsRequest,
    responses((status = 200, body = ServerDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn update_settings(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateSettingsRequest>,
) -> Result<Json<ServerDto>, ApiError> {
    session.require(&iam, Permission::ServersUpdate).await?;
    let view = servers::update_settings(&state, &session.context, id, &request.settings).await?;
    Ok(Json(dto(&state, view)))
}

/// Modifie les ressources. Mémoire, CPU et quota disque s'appliquent à chaud, sans
/// redémarrage ; le nœud doit être en ligne. Permission : `servers.update`.
#[utoipa::path(patch, path = "/api/admin/v1/servers/{id}/resources", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = UpdateResourcesRequest,
    responses((status = 200, body = ServerDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem),
        (status = 422, description = "Hors bornes, au-delà d'un plafond, ou rien à modifier", body = Problem),
        (status = 502, body = Problem)))]
pub async fn update_resources(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<UpdateResourcesRequest>,
) -> Result<Json<ServerDto>, ApiError> {
    session.require(&iam, Permission::ServersUpdate).await?;
    let patch = servers::ResourcesPatch {
        memory_mb: request.memory_mb,
        cpu_percent: request.cpu_percent,
        disk_mb: request.disk_mb,
        backup_limit: request.backup_limit,
    };
    let view = servers::update_resources(&state, &session.context, id, patch).await?;
    Ok(Json(dto(&state, view)))
}

/// Suspend le serveur : le nœud l'arrête, puis refuse démarrage, fichiers, console et
/// sauvegardes jusqu'à la réactivation. Permission : `servers.suspend`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/suspend", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ServerDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn suspend(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<ServerDto>, ApiError> {
    session.require(&iam, Permission::ServersSuspend).await?;
    let view = servers::set_suspended(&state, &session.context, id, true).await?;
    Ok(Json(dto(&state, view)))
}

/// Réactive un serveur suspendu (il reste arrêté). Permission : `servers.suspend`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/unsuspend", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ServerDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn unsuspend(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<ServerDto>, ApiError> {
    session.require(&iam, Permission::ServersSuspend).await?;
    let view = servers::set_suspended(&state, &session.context, id, false).await?;
    Ok(Json(dto(&state, view)))
}

/// Démarrer, arrêter, redémarrer, tuer. Accepté par le nœud ; l'état suit.
/// Permission : `servers.power`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/power", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = PowerRequest,
    responses((status = 202), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn power(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<PowerRequest>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::ServersPower).await?;
    let action = match request.action {
        PowerActionDto::Start => PowerAction::Start,
        PowerActionDto::Stop => PowerAction::Stop,
        PowerActionDto::Restart => PowerAction::Restart,
        PowerActionDto::Kill => PowerAction::Kill,
    };
    servers::power(&state, &session.context, id, action).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Réinstalle ou réinitialise le serveur (arrêté d'abord). Accepté par le nœud ; l'état suit
/// (`installing`, puis `offline`). Permission : `servers.reinstall`.
#[utoipa::path(post, path = "/api/admin/v1/servers/{id}/reinstall", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = ReinstallRequest,
    responses((status = 202), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn reinstall(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<ReinstallRequest>,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::ServersReinstall)
        .await?;
    let mode = match request.mode {
        ReinstallModeDto::Reinstall => InstallMode::Reinstall,
        ReinstallModeDto::Reset => InstallMode::Reset,
    };
    servers::reinstall(&state, &session.context, id, mode).await?;
    Ok(StatusCode::ACCEPTED)
}

/// Supprime le serveur et ses données sur le nœud ; il disparaît quand le nœud confirme.
/// Permission : `servers.delete`.
#[utoipa::path(delete, path = "/api/admin/v1/servers/{id}", tag = "admin-servers", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 202), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::ServersDelete)
        .await?;
    servers::delete(&state, &session.context, id).await?;
    Ok(StatusCode::ACCEPTED)
}
