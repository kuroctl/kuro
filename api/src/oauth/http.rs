//! Serveur d'autorisation OAuth du MCP (ADR-0023).
//!
//! - Public, à la racine : découverte (`/.well-known/…`), `/oauth/authorize`, `/oauth/token`,
//!   `/oauth/register`, `/oauth/revoke`. Réponses au format OAuth (`error`), pas problem+json.
//! - Espace admin : page de consentement et applications connectées, pour l'admin connecté.
//!   Pas de permission dédiée : on ne délègue que ses propres droits.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get as route_get, post as route_post};
use axum::{Form, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use tower_http::cors::{Any, CorsLayer};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use super::application::{
    self as oauth, Authorize, AuthorizeParams, ConsentError, OAuthError, OAuthState, TokenParams,
};
use super::domain::{ClientMetadata, scope_string, supported_scopes, with_query};
use crate::http::context::RequestContext;
use crate::http::error::{ApiError, ErrorCode, Problem, internal_error};
use crate::http::extract::{Json, Path, Query};
use crate::iam::http::AdminSession;

#[derive(OpenApi)]
#[openapi(
    paths(consent_request, approve, deny, list_grants, revoke_grant),
    tags((name = "admin-oauth", description = "Agents IA : consentement et applications connectées (ADR-0023)"))
)]
pub struct OAuthApi;

/// Serveur d'autorisation configuré (`mcp.public_url`) **et** activé dans les paramètres ;
/// sinon 404. La bascule est relue à chaque requête : elle prend effet aussitôt.
pub struct Enabled(pub OAuthState);

impl<S> FromRequestParts<S> for Enabled
where
    Option<OAuthState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Configured(oauth) = Configured::from_request_parts(parts, state).await?;
        match oauth.enabled().await {
            Ok(true) => Ok(Self(oauth)),
            Ok(false) => Err(not_found()),
            Err(error) => Err(internal_error(&error, "failed to read the mcp setting")),
        }
    }
}

/// Serveur d'autorisation configuré, activé ou non : un admin peut toujours lister et
/// révoquer ses accès.
pub struct Configured(pub OAuthState);

impl<S> FromRequestParts<S> for Configured
where
    Option<OAuthState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(_parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Option::<OAuthState>::from_ref(state)
            .map(Self)
            .ok_or_else(not_found)
    }
}

fn not_found() -> Response {
    Problem::new(StatusCode::NOT_FOUND, ErrorCode::NotFound).into_response()
}

/// Routes publiques, montées à la racine. Clients navigateur compris : CORS ouvert, sans
/// cookie (les jetons passent dans le corps ou l'en-tête `Authorization`).
pub fn public_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Option<OAuthState>: FromRef<S>,
{
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            route_get(protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            route_get(protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            route_get(authorization_server),
        )
        .route("/oauth/token", route_post(token))
        .route("/oauth/register", route_post(register))
        .route("/oauth/revoke", route_post(revoke))
        .layer(cors)
        .route("/oauth/authorize", route_get(authorize))
        // Sans session admin (et `authorize` déclenche une lecture de métadonnées distante) :
        // débit limité par adresse cliente.
        .route_layer(axum::middleware::from_fn(
            crate::http::rate_limit::limit_unauthenticated,
        ))
}

/// Routes de l'espace admin (`/api/admin/v1`).
pub fn admin_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Option<OAuthState>: FromRef<S>,
    crate::iam::application::IamState: FromRef<S>,
{
    Router::new()
        .route("/oauth/requests/{id}", route_get(consent_request))
        .route("/oauth/requests/{id}/approve", route_post(approve))
        .route("/oauth/requests/{id}/deny", route_post(deny))
        .route("/oauth/grants", route_get(list_grants))
        .route("/oauth/grants/{id}", axum::routing::delete(revoke_grant))
}

/// Réponse d'erreur OAuth (RFC 6749 §5.2).
fn oauth_error(error: &OAuthError) -> Response {
    let status = match error {
        OAuthError::InvalidClient(_) => StatusCode::UNAUTHORIZED,
        OAuthError::SlowDown => StatusCode::TOO_MANY_REQUESTS,
        OAuthError::Database(_) | OAuthError::Internal(_) => {
            return internal_error(error, "oauth request failed");
        }
        _ => StatusCode::BAD_REQUEST,
    };
    let mut body = json!({ "error": error.code() });
    if let Some(description) = error.description() {
        body["error_description"] = description.into();
    }
    no_store((status, axum::Json(body)).into_response())
}

/// Les réponses qui portent des jetons ne sont jamais mises en cache (RFC 6749 §5.1).
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// RFC 9728 : métadonnées de la ressource protégée `/mcp`.
async fn protected_resource(Enabled(state): Enabled) -> Response {
    let scopes: Vec<&str> = supported_scopes().iter().map(|p| p.key()).collect();
    axum::Json(json!({
        "resource": state.resource(),
        "authorization_servers": [state.issuer],
        "scopes_supported": scopes,
        "bearer_methods_supported": ["header"],
        "resource_name": "Kuro",
    }))
    .into_response()
}

/// RFC 8414 : métadonnées du serveur d'autorisation.
async fn authorization_server(Enabled(state): Enabled) -> Response {
    let issuer = &state.issuer;
    let scopes: Vec<&str> = supported_scopes().iter().map(|p| p.key()).collect();
    axum::Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "registration_endpoint": format!("{issuer}/oauth/register"),
        "revocation_endpoint": format!("{issuer}/oauth/revoke"),
        "scopes_supported": scopes,
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    }))
    .into_response()
}

/// Demande d'autorisation : vers la page de consentement du panel, ou erreur.
async fn authorize(Enabled(state): Enabled, Query(params): Query<AuthorizeParams>) -> Response {
    let consent = format!("{}/oauth/consent", state.panel_url);
    match oauth::authorize(&state, params).await {
        Ok(Authorize::Consent(id)) => {
            Redirect::to(&with_query(&consent, &[("request", &id.to_string())])).into_response()
        }
        Ok(Authorize::Refused(reason)) => {
            Redirect::to(&with_query(&consent, &[("error", reason)])).into_response()
        }
        Ok(Authorize::Redirect(uri)) => Redirect::to(&uri).into_response(),
        Err(error) => oauth_error(&error),
    }
}

async fn token(
    Enabled(state): Enabled,
    RequestContext(context): RequestContext,
    Form(params): Form<TokenParams>,
) -> Response {
    match oauth::token(&state, &context, params).await {
        Ok(tokens) => no_store(axum::Json(tokens).into_response()),
        Err(error) => oauth_error(&error),
    }
}

async fn register(
    Enabled(state): Enabled,
    RequestContext(context): RequestContext,
    axum::Json(metadata): axum::Json<ClientMetadata>,
) -> Response {
    match oauth::register(&state, &context, metadata).await {
        Ok(client) => no_store((StatusCode::CREATED, axum::Json(client)).into_response()),
        Err(error) => oauth_error(&error),
    }
}

#[derive(Debug, Deserialize)]
struct RevokeParams {
    token: Option<String>,
    client_id: Option<String>,
}

async fn revoke(
    Enabled(state): Enabled,
    RequestContext(context): RequestContext,
    Form(params): Form<RevokeParams>,
) -> Response {
    let Some(token) = params.token else {
        return oauth_error(&OAuthError::InvalidRequest("token is required"));
    };
    match oauth::revoke(&state, &context, &token, params.client_id.as_deref()).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => oauth_error(&error),
    }
}

impl From<ConsentError> for ApiError {
    fn from(error: ConsentError) -> Self {
        match error {
            ConsentError::NotFound => {
                Problem::new(StatusCode::NOT_FOUND, ErrorCode::NotFound).into()
            }
            ConsentError::NoScope => {
                Problem::new(StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::OauthNoScope).into()
            }
            ConsentError::Database(error) => internal_error(&error, "oauth consent failed").into(),
            ConsentError::Internal(error) => internal_error(&error, "oauth consent failed").into(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentScopeDto {
    /// Clé de permission (`servers.power`).
    pub key: String,
    pub sensitive: bool,
    /// L'admin connecté la détient : sinon elle ne sera pas accordée.
    pub held: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientRegistrationDto {
    /// Document de métadonnées lu à l'URL du `client_id`.
    Metadata,
    /// Enregistré dynamiquement : rien ne le rattache à un domaine.
    Dynamic,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentClientDto {
    /// Nom déclaré par le client, non vérifié.
    pub name: String,
    pub client_id: String,
    pub registration: ClientRegistrationDto,
    pub uri: Option<String>,
    /// Hôte de l'URI de retour : là où partira le code.
    pub redirect_host: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentRequestDto {
    pub id: Uuid,
    pub client: ConsentClientDto,
    pub scopes: Vec<ConsentScopeDto>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: OffsetDateTime,
    /// Durée d'un jeton d'accès et du jeton de rafraîchissement, en secondes.
    pub access_ttl_secs: u64,
    pub refresh_ttl_secs: u64,
}

fn registration(kind: &str) -> ClientRegistrationDto {
    if kind == "metadata" {
        ClientRegistrationDto::Metadata
    } else {
        ClientRegistrationDto::Dynamic
    }
}

/// Demande d'autorisation en attente, pour la page de consentement.
#[utoipa::path(get, path = "/api/admin/v1/oauth/requests/{id}", tag = "admin-oauth", security(("bearer" = [])),
    responses((status = 200, body = ConsentRequestDto), (status = 404, description = "Inconnue, expirée ou déjà traitée", body = Problem)))]
pub async fn consent_request(
    Enabled(state): Enabled,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<ConsentRequestDto>, ApiError> {
    let view = oauth::consent_view(&state, &session.principal, id).await?;
    let request = view.request;
    let redirect_host = request
        .redirect_uri
        .parse::<hyper::Uri>()
        .ok()
        .and_then(|uri| uri.host().map(str::to_owned))
        .unwrap_or_default();
    Ok(Json(ConsentRequestDto {
        id: request.id,
        client: ConsentClientDto {
            name: request.client_name,
            client_id: request.client_id,
            registration: registration(&request.registration),
            uri: request.client_uri,
            redirect_host,
        },
        scopes: view
            .scopes
            .iter()
            .map(|scope| ConsentScopeDto {
                key: scope.permission.key().to_owned(),
                sensitive: scope.permission.is_sensitive(),
                held: scope.held,
            })
            .collect(),
        expires_at: request.expires_at,
        access_ttl_secs: super::domain::ACCESS_TTL.as_secs(),
        refresh_ttl_secs: super::domain::REFRESH_TTL.as_secs(),
    }))
}

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ApproveRequest {
    /// Droits retenus parmi ceux demandés ; absent : tous ceux que l'admin détient.
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
}

/// Où renvoyer le navigateur : l'URI de retour du client, avec le code ou l'erreur.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConsentRedirectDto {
    pub redirect_to: String,
}

/// Accorde la demande : crée l'accord (remplace celui du même client) et émet un code.
#[utoipa::path(post, path = "/api/admin/v1/oauth/requests/{id}/approve", tag = "admin-oauth", security(("bearer" = [])),
    request_body = ApproveRequest,
    responses((status = 200, body = ConsentRedirectDto), (status = 404, body = Problem),
        (status = 422, description = "Aucun droit accordable (`oauth_no_scope`)", body = Problem)))]
pub async fn approve(
    Enabled(state): Enabled,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<ApproveRequest>,
) -> Result<Json<ConsentRedirectDto>, ApiError> {
    let redirect_to = oauth::approve(
        &state,
        &session.principal,
        &session.context,
        id,
        request.scopes,
    )
    .await?;
    Ok(Json(ConsentRedirectDto { redirect_to }))
}

/// Refuse la demande : le client reçoit `access_denied`.
#[utoipa::path(post, path = "/api/admin/v1/oauth/requests/{id}/deny", tag = "admin-oauth", security(("bearer" = [])),
    responses((status = 200, body = ConsentRedirectDto), (status = 404, body = Problem)))]
pub async fn deny(
    Enabled(state): Enabled,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<ConsentRedirectDto>, ApiError> {
    let redirect_to = oauth::deny(&state, &session.context, id).await?;
    Ok(Json(ConsentRedirectDto { redirect_to }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GrantDto {
    pub id: Uuid,
    pub client_name: String,
    pub client_id: String,
    pub registration: ClientRegistrationDto,
    pub scopes: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub last_used_at: Option<OffsetDateTime>,
}

/// Applications connectées de l'admin : accès accordés à des agents IA.
#[utoipa::path(get, path = "/api/admin/v1/oauth/grants", tag = "admin-oauth", security(("bearer" = [])),
    responses((status = 200, body = [GrantDto])))]
pub async fn list_grants(
    Configured(state): Configured,
    session: AdminSession,
) -> Result<Json<Vec<GrantDto>>, ApiError> {
    let grants = oauth::grants(&state, &session.principal).await?;
    Ok(Json(
        grants
            .into_iter()
            .map(|grant| GrantDto {
                id: grant.id,
                client_name: grant.client_name,
                client_id: grant.client_id,
                registration: registration(&grant.registration),
                scopes: grant.scopes,
                created_at: grant.created_at,
                last_used_at: grant.last_used_at,
            })
            .collect(),
    ))
}

/// Révoque un accès : ses jetons cessent aussitôt de fonctionner.
#[utoipa::path(delete, path = "/api/admin/v1/oauth/grants/{id}", tag = "admin-oauth", security(("bearer" = [])),
    responses((status = 204), (status = 404, body = Problem)))]
pub async fn revoke_grant(
    Configured(state): Configured,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    oauth::revoke_grant(&state, &session.principal, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `WWW-Authenticate` d'une requête MCP sans jeton valide (401) ou sans le droit requis (403).
pub fn challenge(state: &OAuthState, insufficient: Option<&str>) -> HeaderValue {
    let metadata = format!("{}/.well-known/oauth-protected-resource/mcp", state.issuer);
    let value = match insufficient {
        Some(scope) => format!(
            r#"Bearer error="insufficient_scope", scope="{scope}", resource_metadata="{metadata}""#
        ),
        None => format!(
            r#"Bearer resource_metadata="{metadata}", scope="{}""#,
            scope_string(&supported_scopes())
        ),
    };
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static("Bearer"))
}
