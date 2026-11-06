//! Nœuds à la demande (ADR-0065) : réglages (`system.settings.*`), machines (`nodes.*`).

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

use super::application::{
    self as machines, MachineView, MachinesError, MachinesSettings, MachinesState, OfferChoice,
    PendingServer, ProviderSaved, ProviderStatus, RegionHosting, RegionHostingInput, RentInput,
    SettingsInput,
};
use super::provider::{Location, Offer, ProviderError};
use crate::error_code::{ErrorCode, FieldError};
use crate::http::error::{ApiError, Problem, internal_error};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::tasks::{TaskStatus, Tasks};

#[derive(OpenApi)]
#[openapi(
    paths(get_settings, put_settings, put_provider, delete_provider, provider_locations, provider_regions, list, offers, rent, release, sleep, wake),
    components(schemas(MachinesSettings, ProviderStatus, RegionHosting, RegionHostingInput, MachineView, Offer, Location, ProviderSaved)),
    tags((name = "admin-machines", description = "Nœuds à la demande (ADR-0065)"))
)]
pub struct MachinesApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    MachinesState: FromRef<S>,
    Tasks: FromRef<S>,
{
    Router::new()
        .route("/settings/machines", get(get_settings).put(put_settings))
        .route(
            "/settings/machines/providers/{provider}",
            put(put_provider).delete(delete_provider),
        )
        .route(
            "/settings/machines/providers/{provider}/locations",
            get(provider_locations),
        )
        .route(
            "/settings/machines/providers/{provider}/regions",
            post(provider_regions),
        )
        .route("/machines", get(list).post(rent))
        .route("/machines/offers", get(offers))
        .route("/machines/{id}", axum::routing::delete(release))
        .route("/machines/{id}/sleep", post(sleep))
        .route("/machines/{id}/wake", post(wake))
}

impl From<MachinesError> for ApiError {
    fn from(error: MachinesError) -> Self {
        let (status, code) = match error {
            MachinesError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            MachinesError::Invalid(field) => return Problem::invalid(field).into(),
            MachinesError::Unprocessable(code) => (StatusCode::UNPROCESSABLE_ENTITY, code),
            MachinesError::Conflict(code) => (StatusCode::CONFLICT, code),
            MachinesError::Nodes(error) => return error.into(),
            MachinesError::Server(error) => return error.into(),
            // Le motif du fournisseur n'est vu que par l'admin : paramètres du problème.
            MachinesError::Provider(ProviderError::Refused { code, message }) => {
                return Problem::invalid(
                    FieldError::new("provider", ErrorCode::MachineProviderRefused)
                        .param("code", code)
                        .param("message", message),
                )
                .into();
            }
            MachinesError::Provider(error @ ProviderError::Transport(_)) => {
                tracing::warn!(%error, "machine provider failed");
                (
                    StatusCode::BAD_GATEWAY,
                    ErrorCode::MachineProviderUnavailable,
                )
            }
            error @ (MachinesError::Database(_) | MachinesError::Internal(_)) => {
                return internal_error(&error, "machines request failed").into();
            }
        };
        Problem::new(status, code).into()
    }
}

/// Réglages des machines. Permission : `system.settings.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/machines", tag = "admin-machines", security(("bearer" = [])),
    responses((status = 200, body = MachinesSettings), (status = 403, body = Problem)))]
pub async fn get_settings(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
) -> Result<Json<MachinesSettings>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsRead)
        .await?;
    Ok(Json(machines::settings(&state.pool).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MachinesSettingsRequest {
    /// Adresse de l'API vue par les machines ; vide : aucune location possible.
    pub api_url: Option<String>,
    /// Ports des serveurs (`25565-25600`).
    pub port_range: String,
    /// Fournisseur et emplacement de chaque région louée ; les autres ne louent rien.
    #[serde(default)]
    pub regions: Vec<RegionHostingInput>,
}

/// Adresse de l'API, ports, régions louées. Permission : `system.settings.update`.
#[utoipa::path(put, path = "/api/admin/v1/settings/machines", tag = "admin-machines", security(("bearer" = [])),
    request_body = MachinesSettingsRequest,
    responses((status = 200, body = MachinesSettings), (status = 403, body = Problem), (status = 422, body = Problem)))]
pub async fn put_settings(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Json(request): Json<MachinesSettingsRequest>,
) -> Result<Json<MachinesSettings>, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSettingsUpdate)
        .await?;
    Ok(Json(
        machines::update_settings(
            &state.pool,
            &session.context,
            SettingsInput {
                api_url: request.api_url,
                port_range: request.port_range,
                regions: request.regions,
            },
        )
        .await?,
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ProviderRequest {
    /// Jeton d'API du fournisseur ; essayé avant d'être enregistré, jamais renvoyé.
    pub token: String,
}

/// Crée et relie une région (désactivée) par emplacement d'un fournisseur configuré, comme à
/// l'enregistrement du jeton. Permission : `system.settings.update`.
#[utoipa::path(post, path = "/api/admin/v1/settings/machines/providers/{provider}/regions", tag = "admin-machines", security(("bearer" = [])),
    params(("provider" = String, Path)),
    responses((status = 200, body = ProviderSaved), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn provider_regions(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(provider): Path<String>,
) -> Result<Json<ProviderSaved>, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSettingsUpdate)
        .await?;
    Ok(Json(
        machines::create_regions(&state, &session.context, &provider).await?,
    ))
}

/// Emplacements d'un fournisseur configuré. Permission : `system.settings.read`.
#[utoipa::path(get, path = "/api/admin/v1/settings/machines/providers/{provider}/locations", tag = "admin-machines", security(("bearer" = [])),
    params(("provider" = String, Path)),
    responses((status = 200, body = Vec<Location>), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn provider_locations(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(provider): Path<String>,
) -> Result<Json<Vec<Location>>, ApiError> {
    session
        .require(&iam, Permission::SystemSettingsRead)
        .await?;
    Ok(Json(machines::locations(&state, &provider).await?))
}

/// Identifiants d'un fournisseur (`hetzner`) ; crée et relie une région (désactivée) par
/// emplacement. Permission : `system.settings.update`.
#[utoipa::path(put, path = "/api/admin/v1/settings/machines/providers/{provider}", tag = "admin-machines", security(("bearer" = [])),
    params(("provider" = String, Path)), request_body = ProviderRequest,
    responses((status = 200, body = ProviderSaved), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn put_provider(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(provider): Path<String>,
    Json(request): Json<ProviderRequest>,
) -> Result<Json<ProviderSaved>, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSettingsUpdate)
        .await?;
    Ok(Json(
        machines::set_provider(&state, &session.context, &provider, &request.token).await?,
    ))
}

/// Retire les identifiants d'un fournisseur inutilisé. Permission : `system.settings.update`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/machines/providers/{provider}", tag = "admin-machines", security(("bearer" = [])),
    params(("provider" = String, Path)),
    responses((status = 200, body = MachinesSettings), (status = 403, body = Problem), (status = 404, body = Problem), (status = 409, body = Problem)))]
pub async fn delete_provider(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(provider): Path<String>,
) -> Result<Json<MachinesSettings>, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemSettingsUpdate)
        .await?;
    Ok(Json(
        machines::remove_provider(&state.pool, &session.context, &provider).await?,
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MachinesPage {
    /// Au moins un fournisseur a des identifiants : sinon, aller aux Réglages d'abord.
    pub configured: bool,
    pub machines: Vec<MachineView>,
    /// Dernier passage de ménage (`machines.sweep`).
    pub sweep: Option<TaskStatus>,
}

/// Machines louées. Permission : `nodes.read`.
#[utoipa::path(get, path = "/api/admin/v1/machines", tag = "admin-machines", security(("bearer" = [])),
    responses((status = 200, body = MachinesPage), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    State(tasks): State<Tasks>,
    session: AdminSession,
) -> Result<Json<MachinesPage>, ApiError> {
    session.require(&iam, Permission::NodesRead).await?;
    Ok(Json(MachinesPage {
        configured: machines::any_provider(&state.pool)
            .await
            .map_err(MachinesError::from)?,
        machines: machines::list(&state.pool, None).await?,
        sweep: tasks.status("machines.sweep").await.ok(),
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct OffersQuery {
    pub region_id: Uuid,
}

/// Offres du fournisseur d'une région. Permission : `nodes.create`.
#[utoipa::path(get, path = "/api/admin/v1/machines/offers", tag = "admin-machines", security(("bearer" = [])),
    params(OffersQuery),
    responses((status = 200, body = Vec<Offer>), (status = 403, body = Problem), (status = 409, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn offers(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Query(query): Query<OffersQuery>,
) -> Result<Json<Vec<Offer>>, ApiError> {
    session.require(&iam, Permission::NodesCreate).await?;
    Ok(Json(machines::offers(&state, query.region_id).await?))
}

/// Serveur à créer sur le nœud de la machine dès qu'il est en ligne (mêmes champs que la
/// création d'un serveur, sans nœud ni allocations).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PendingServerRequest {
    pub name: String,
    pub prefab_id: String,
    pub prefab_version: Option<String>,
    pub memory_mb: Option<i32>,
    pub cpu_percent: Option<i32>,
    pub disk_mb: Option<i32>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub settings: Map<String, Value>,
    #[serde(default)]
    pub start_when_installed: bool,
    pub backup_limit: Option<i32>,
    pub owner_ref: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RentRequest {
    pub region_id: Uuid,
    /// Offre du fournisseur (`cx23`).
    pub offer: String,
    /// Appelant : la machine et son nœud lui sont réservés.
    pub claim: Option<String>,
    /// Serveur à créer sur la machine (`servers.create` en plus).
    pub server: Option<PendingServerRequest>,
}

/// Loue une machine, enrôlée comme nœud. Permission : `nodes.create` (et `servers.create`,
/// `servers.power` pour le serveur attendu).
#[utoipa::path(post, path = "/api/admin/v1/machines", tag = "admin-machines", security(("bearer" = [])),
    request_body = RentRequest,
    responses((status = 201, body = MachineView), (status = 403, body = Problem), (status = 409, body = Problem),
        (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn rent(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Json(request): Json<RentRequest>,
) -> Result<(StatusCode, Json<MachineView>), ApiError> {
    session.require(&iam, Permission::NodesCreate).await?;
    if let Some(server) = &request.server {
        session.require(&iam, Permission::ServersCreate).await?;
        if server.start_when_installed {
            session.require(&iam, Permission::ServersPower).await?;
        }
        if !crate::validation::is_display_text(server.name.trim(), 1, 64) {
            return Err(Problem::invalid(FieldError::new(
                "server.name",
                ErrorCode::InvalidServerName,
            ))
            .into());
        }
    }
    let machine = machines::rent(
        &state,
        &session.context,
        RentInput {
            region_id: request.region_id,
            offer: OfferChoice::Named(request.offer),
            claim: request.claim,
            pending_server: request.server.map(|server| PendingServer {
                name: server.name.trim().to_owned(),
                prefab_id: server.prefab_id,
                prefab_version: server.prefab_version,
                memory_mb: server.memory_mb,
                cpu_percent: server.cpu_percent,
                disk_mb: server.disk_mb,
                settings: server.settings,
                start_when_installed: server.start_when_installed,
                backup_limit: server.backup_limit,
                owner_ref: server.owner_ref,
            }),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(machine)))
}

/// Rend une machine (VM, snapshot et nœud). Permission : `nodes.delete`.
#[utoipa::path(delete, path = "/api/admin/v1/machines/{id}", tag = "admin-machines", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem), (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn release(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::NodesDelete).await?;
    machines::release(&state, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Met une machine en sommeil (snapshot, puis VM rendue). Permission : `nodes.update`.
#[utoipa::path(post, path = "/api/admin/v1/machines/{id}/sleep", tag = "admin-machines", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = MachineView), (status = 403, body = Problem), (status = 404, body = Problem), (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn sleep(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<MachineView>, ApiError> {
    session.require(&iam, Permission::NodesUpdate).await?;
    Ok(Json(machines::sleep(&state, &session.context, id).await?))
}

/// Réveille une machine depuis son snapshot. Permission : `nodes.update`.
#[utoipa::path(post, path = "/api/admin/v1/machines/{id}/wake", tag = "admin-machines", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = MachineView), (status = 403, body = Problem), (status = 404, body = Problem), (status = 409, body = Problem), (status = 502, body = Problem)))]
pub async fn wake(
    State(iam): State<IamState>,
    State(state): State<MachinesState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<MachineView>, ApiError> {
    session.require(&iam, Permission::NodesUpdate).await?;
    Ok(Json(machines::wake(&state, &session.context, id).await?))
}
