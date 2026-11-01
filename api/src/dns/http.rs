//! DNS (ADR-0026) : zone dans les paramètres (`system.dns.manage`), label d'un serveur
//! (`servers.update`), adresses publiques d'un nœud (`nodes.update`), zone publique (pour
//! composer l'adresse d'un serveur, `servers.read`).

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use super::application::{self as dns, DnsError, DnsState, ZoneInput};
use super::infra::DnsRepository;
use super::provider::ProviderError;
use crate::error_code::{ErrorCode, FieldError};
use crate::http::error::{ApiError, Problem, internal_error};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(public_zone, get_settings, put_settings, delete_settings, test_settings, sync_now, set_label, set_address),
    tags((name = "admin-dns", description = "DNS des serveurs et des nœuds (ADR-0026)"))
)]
pub struct DnsApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    DnsState: FromRef<S>,
{
    Router::new()
        .route("/dns", get(public_zone))
        .route(
            "/settings/dns",
            get(get_settings).put(put_settings).delete(delete_settings),
        )
        .route("/settings/dns/test", post(test_settings))
        .route("/settings/dns/sync", post(sync_now))
        .route("/servers/{id}/dns", put(set_label))
        .route("/nodes/{id}/address", put(set_address))
}

impl From<DnsError> for ApiError {
    fn from(error: DnsError) -> Self {
        let (status, code) = match &error {
            DnsError::NotConfigured => (StatusCode::NOT_FOUND, ErrorCode::DnsNotConfigured),
            DnsError::NotFound => (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            DnsError::InvalidDomain => {
                return Problem::invalid(FieldError::new("base_domain", ErrorCode::InvalidDomain))
                    .into();
            }
            DnsError::InvalidLabel => {
                return Problem::invalid(FieldError::new("label", ErrorCode::InvalidDnsLabel))
                    .into();
            }
            DnsError::InvalidSitesDomain => {
                return Problem::invalid(FieldError::new(
                    "sites_domain",
                    ErrorCode::InvalidSitesDomain,
                ))
                .into();
            }
            DnsError::LabelTaken => {
                return Problem::invalid(FieldError::new("label", ErrorCode::DnsLabelTaken)).into();
            }
            DnsError::LabelReserved => {
                return Problem::invalid(FieldError::new("label", ErrorCode::DnsLabelReserved))
                    .into();
            }
            DnsError::InvalidAddress => {
                return Problem::invalid(FieldError::new("address", ErrorCode::InvalidIpAddress))
                    .into();
            }
            DnsError::TokenRequired => {
                return Problem::invalid(FieldError::new("token", ErrorCode::DnsTokenRequired))
                    .into();
            }
            DnsError::UnknownProvider => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::UnknownDnsProvider,
            ),
            DnsError::Provider(ProviderError::Unauthorized) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::DnsProviderUnauthorized,
            ),
            DnsError::Provider(ProviderError::ZoneNotFound) => {
                (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::DnsZoneNotFound)
            }
            DnsError::Provider(ProviderError::Unavailable(_) | ProviderError::Rejected(_)) => {
                tracing::warn!(%error, "dns provider failed");
                (StatusCode::BAD_GATEWAY, ErrorCode::DnsProviderUnavailable)
            }
            DnsError::Database(_) | DnsError::Internal(_) => {
                return internal_error(&error, "dns request failed").into();
            }
        };
        Problem::new(status, code).into()
    }
}

/// Zone vue par tous : de quoi composer l'adresse d'un serveur, jamais l'accès au fournisseur.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicZoneDto {
    /// `play.kuroctl.com` ; absent sans DNS configuré ou activé.
    pub base_domain: Option<String>,
    pub nodes_label: Option<String>,
    /// Domaine des sites web (`web.kuroctl.com`, ADR-0038) ; absent : aucun site publié.
    pub sites_domain: Option<String>,
}

/// Domaine des serveurs. Permission : `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/dns", tag = "admin-dns", security(("bearer" = [])),
    responses((status = 200, body = PublicZoneDto)))]
pub async fn public_zone(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
) -> Result<Json<PublicZoneDto>, ApiError> {
    session.require(&iam, Permission::ServersRead).await?;
    let zone = dns::zone(&state).await?.filter(|z| z.enabled);
    Ok(Json(PublicZoneDto {
        base_domain: zone.as_ref().map(|z| z.base_domain.clone()),
        nodes_label: zone.as_ref().map(|z| z.nodes_label.clone()),
        sites_domain: zone.and_then(|z| z.sites_domain),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DnsRecordDto {
    pub id: Uuid,
    /// `node` ou `server`.
    pub owner_kind: String,
    pub owner_id: Uuid,
    /// `A`, `AAAA`, `CNAME`, `SRV`.
    pub kind: String,
    pub name: String,
    pub content: String,
    /// Publié chez le fournisseur.
    pub published: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub synced_at: Option<OffsetDateTime>,
    /// Dernier échec, réessayé à la passe suivante.
    pub error: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DnsSettingsDto {
    pub configured: bool,
    pub provider: Option<String>,
    pub base_domain: Option<String>,
    pub nodes_label: Option<String>,
    /// Domaine des sites web (ADR-0038) ; absent : aucun prefab web n'est déployable.
    pub sites_domain: Option<String>,
    pub ttl: Option<i32>,
    pub enabled: bool,
    /// Un jeton est enregistré ; sa valeur n'est jamais renvoyée.
    pub has_token: bool,
    pub records: Vec<DnsRecordDto>,
}

async fn settings_dto(state: &DnsState) -> Result<DnsSettingsDto, ApiError> {
    let zone = dns::zone(state).await?;
    let records = match &zone {
        Some(zone) => {
            let mut conn = state.pool.acquire().await.map_err(DnsError::from)?;
            DnsRepository::records(&mut conn, zone.id)
                .await
                .map_err(DnsError::from)?
        }
        None => Vec::new(),
    };
    Ok(DnsSettingsDto {
        configured: zone.is_some(),
        provider: zone.as_ref().map(|z| z.provider.clone()),
        base_domain: zone.as_ref().map(|z| z.base_domain.clone()),
        nodes_label: zone.as_ref().map(|z| z.nodes_label.clone()),
        sites_domain: zone.as_ref().and_then(|z| z.sites_domain.clone()),
        ttl: zone.as_ref().map(|z| z.ttl),
        enabled: zone.as_ref().is_some_and(|z| z.enabled),
        has_token: zone.is_some(),
        records: records
            .into_iter()
            .map(|r| DnsRecordDto {
                id: r.id,
                owner_kind: r.owner_kind,
                owner_id: r.owner_id,
                kind: r.kind,
                name: r.name,
                content: r.content,
                published: r.provider_id.is_some(),
                synced_at: r.synced_at,
                error: r.error,
            })
            .collect(),
    })
}

/// Configuration DNS et enregistrements. Permission : `system.dns.manage`.
#[utoipa::path(get, path = "/api/admin/v1/settings/dns", tag = "admin-dns", security(("bearer" = [])),
    responses((status = 200, body = DnsSettingsDto), (status = 403, body = Problem)))]
pub async fn get_settings(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
) -> Result<Json<DnsSettingsDto>, ApiError> {
    session.require(&iam, Permission::SystemDnsManage).await?;
    Ok(Json(settings_dto(&state).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DnsSettingsRequest {
    /// `cloudflare`.
    pub provider: String,
    /// Domaine des serveurs (`play.kuroctl.com`), dans une zone du fournisseur.
    pub base_domain: String,
    /// Sous-domaine des nœuds (`nodes` par défaut).
    #[serde(default)]
    pub nodes_label: Option<String>,
    /// Domaine des sites web (`web.kuroctl.com`, ADR-0038), dans la même zone ; vide ou absent :
    /// aucun prefab web n'est déployable.
    #[serde(default)]
    pub sites_domain: Option<String>,
    /// Jeton d'API ; absent : celui enregistré est gardé.
    #[serde(default)]
    pub token: Option<String>,
    /// 60 à 86400 s, 300 par défaut.
    #[serde(default)]
    pub ttl: Option<i32>,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

/// Enregistre la zone ; le fournisseur doit accepter le jeton et contenir le domaine.
/// Permission : `system.dns.manage`.
#[utoipa::path(put, path = "/api/admin/v1/settings/dns", tag = "admin-dns", security(("bearer" = [])),
    request_body = DnsSettingsRequest,
    responses((status = 200, body = DnsSettingsDto), (status = 403, body = Problem),
        (status = 422, description = "Domaine, jeton ou zone refusés", body = Problem), (status = 502, body = Problem)))]
pub async fn put_settings(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
    Json(request): Json<DnsSettingsRequest>,
) -> Result<Json<DnsSettingsDto>, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemDnsManage)
        .await?;
    dns::configure(
        &state,
        &session.context,
        ZoneInput {
            provider: request.provider,
            base_domain: request.base_domain,
            nodes_label: request.nodes_label.unwrap_or_else(|| "nodes".to_owned()),
            sites_domain: request.sites_domain,
            token: request.token,
            ttl: request.ttl,
            enabled: request.enabled,
        },
    )
    .await?;
    Ok(Json(settings_dto(&state).await?))
}

/// Supprime la zone et les enregistrements créés par Kuro. Permission : `system.dns.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/settings/dns", tag = "admin-dns", security(("bearer" = [])),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_settings(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
) -> Result<StatusCode, ApiError> {
    session
        .require_elevated(&iam, Permission::SystemDnsManage)
        .await?;
    dns::remove(&state, &session.context).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DnsTestDto {
    /// Enregistrements que le jeton voit dans la zone.
    pub records: usize,
}

/// Vérifie que le jeton enregistré ouvre la zone. Permission : `system.dns.manage`.
#[utoipa::path(post, path = "/api/admin/v1/settings/dns/test", tag = "admin-dns", security(("bearer" = [])),
    responses((status = 200, body = DnsTestDto), (status = 403, body = Problem), (status = 422, body = Problem), (status = 502, body = Problem)))]
pub async fn test_settings(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
) -> Result<Json<DnsTestDto>, ApiError> {
    session.require(&iam, Permission::SystemDnsManage).await?;
    Ok(Json(DnsTestDto {
        records: dns::test(&state).await?,
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DnsSyncDto {
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub failed: usize,
}

/// Réconcilie tout de suite. Permission : `system.dns.manage`.
#[utoipa::path(post, path = "/api/admin/v1/settings/dns/sync", tag = "admin-dns", security(("bearer" = [])),
    responses((status = 200, body = DnsSyncDto), (status = 403, body = Problem), (status = 502, body = Problem)))]
pub async fn sync_now(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
) -> Result<Json<DnsSyncDto>, ApiError> {
    session.require(&iam, Permission::SystemDnsManage).await?;
    let report = dns::reconcile(&state).await?;
    Ok(Json(DnsSyncDto {
        created: report.created,
        updated: report.updated,
        deleted: report.deleted,
        failed: report.failed,
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LabelRequest {
    /// `survie` pour `survie.<domaine>` ; `null` : plus de nom.
    pub label: Option<String>,
}

/// Nom d'un serveur dans la zone. Permission : `servers.update`.
#[utoipa::path(put, path = "/api/admin/v1/servers/{id}/dns", tag = "admin-dns", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = LabelRequest,
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Nom invalide ou déjà pris", body = Problem)))]
pub async fn set_label(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<LabelRequest>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::ServersUpdate).await?;
    let label = request.label.map(|l| l.trim().to_ascii_lowercase());
    dns::set_label(&state, &session.context, id, label.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddressRequest {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

/// Adresses publiques d'un nœud, publiées sous son nom. Permission : `nodes.update`.
#[utoipa::path(put, path = "/api/admin/v1/nodes/{id}/address", tag = "admin-dns", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    request_body = AddressRequest,
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem), (status = 422, body = Problem)))]
pub async fn set_address(
    State(iam): State<IamState>,
    State(state): State<DnsState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<AddressRequest>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::NodesUpdate).await?;
    dns::set_node_addresses(
        &state,
        &session.context,
        id,
        request.ipv4.as_deref(),
        request.ipv6.as_deref(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
