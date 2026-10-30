//! Cas d'usage du serveur d'autorisation : demande, consentement, jetons, révocation.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::domain::{
    ACCESS_PREFIX, ACCESS_TTL, CODE_PREFIX, CODE_TTL, ClientMetadata, DYNAMIC_CLIENT_TTL,
    METADATA_TTL, REFRESH_PREFIX, REFRESH_TTL, REQUEST_TTL, metadata_url_allowed, pkce_matches,
    redirect_uri_matches, requested_scopes, scope_string, valid_challenge, valid_state, with_query,
};
use super::infra::{
    ClientRecord, GrantRecord, NewRequest, OAuthRepository, RequestRecord, TokenKind,
};
use super::metadata::MetadataFetcher;
use crate::audit::domain::{Actor, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::opaque;
use crate::iam::domain::{Permission, PermissionSet, Principal};
use crate::iam::infra::UserRepository;
use crate::settings::infra::SettingsRepository;

/// Enregistrements dynamiques acceptés par adresse et par heure.
const REGISTRATIONS_PER_HOUR: u32 = 10;

/// Serveur d'autorisation configuré (`mcp.public_url`).
#[derive(Debug, Clone)]
pub struct OAuthState {
    pub pool: PgPool,
    /// URL publique de l'API, sans `/` final : émetteur (`iss`) et base des endpoints.
    pub issuer: String,
    /// URL du panel, pour la page de consentement.
    pub panel_url: String,
    pub fetcher: Arc<dyn MetadataFetcher>,
    registrations: Arc<Mutex<HashMap<IpAddr, (Instant, u32)>>>,
}

impl OAuthState {
    /// `public_url` en https, ou en http sur la boucle locale (développement).
    pub fn new(
        pool: PgPool,
        public_url: &str,
        panel_url: Option<&str>,
        fetcher: Arc<dyn MetadataFetcher>,
    ) -> Result<Self, String> {
        let check = |url: &str| -> Result<String, String> {
            let url = url.trim_end_matches('/');
            let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
                .iter()
                .any(|prefix| {
                    url.strip_prefix(prefix)
                        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
                });
            if url.starts_with("https://") || local {
                Ok(url.to_owned())
            } else {
                Err(format!("{url}: https required outside localhost"))
            }
        };
        Ok(Self {
            pool,
            issuer: check(public_url)?,
            panel_url: check(panel_url.unwrap_or(public_url))?,
            fetcher,
            registrations: Arc::default(),
        })
    }

    /// Activé dans les paramètres du projet (`mcp_enabled`).
    pub async fn enabled(&self) -> Result<bool, sqlx::Error> {
        let mut conn = self.pool.acquire().await?;
        SettingsRepository::mcp_enabled(&mut conn).await
    }

    /// Ressource protégée : l'endpoint MCP.
    pub fn resource(&self) -> String {
        format!("{}/mcp", self.issuer)
    }

    /// Débit des enregistrements dynamiques, par adresse.
    fn allow_registration(&self, ip: Option<IpAddr>) -> bool {
        let Some(ip) = ip else { return true };
        let mut map = self
            .registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        map.retain(|_, (since, _)| now.duration_since(*since) < Duration::from_secs(3600));
        let entry = map.entry(ip).or_insert((now, 0));
        entry.1 += 1;
        entry.1 <= REGISTRATIONS_PER_HOUR
    }
}

/// Erreurs du protocole (RFC 6749 §5.2), renvoyées au client.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("invalid_request: {0}")]
    InvalidRequest(&'static str),
    #[error("invalid_client: {0}")]
    InvalidClient(&'static str),
    #[error("invalid_grant: {0}")]
    InvalidGrant(&'static str),
    #[error("unsupported_grant_type")]
    UnsupportedGrantType,
    #[error("invalid_target")]
    InvalidTarget,
    #[error("invalid_client_metadata: {0}")]
    InvalidClientMetadata(&'static str),
    #[error("slow_down")]
    SlowDown,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl OAuthError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::InvalidClient(_) => "invalid_client",
            Self::InvalidGrant(_) => "invalid_grant",
            Self::UnsupportedGrantType => "unsupported_grant_type",
            Self::InvalidTarget => "invalid_target",
            Self::InvalidClientMetadata(_) => "invalid_client_metadata",
            Self::SlowDown => "slow_down",
            Self::Database(_) | Self::Internal(_) => "server_error",
        }
    }

    pub fn description(&self) -> Option<&'static str> {
        match self {
            Self::InvalidRequest(d)
            | Self::InvalidClient(d)
            | Self::InvalidGrant(d)
            | Self::InvalidClientMetadata(d) => Some(d),
            _ => None,
        }
    }
}

/// Erreurs de la page de consentement et de la gestion des accords (espace admin).
#[derive(Debug, thiserror::Error)]
pub enum ConsentError {
    #[error("not found")]
    NotFound,
    #[error("none of the requested rights can be granted")]
    NoScope,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl From<kuro_runtime::tls::TlsError> for OAuthError {
    fn from(error: kuro_runtime::tls::TlsError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<kuro_runtime::tls::TlsError> for ConsentError {
    fn from(error: kuro_runtime::tls::TlsError) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Paramètres de `GET /oauth/authorize`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AuthorizeParams {
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub scope: Option<String>,
    pub resource: Option<String>,
}

/// Suite d'une demande d'autorisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorize {
    /// Demande enregistrée : vers la page de consentement.
    Consent(Uuid),
    /// Client ou URI de retour invalide : affiché par le panel, jamais renvoyé au client.
    Refused(&'static str),
    /// Erreur renvoyée au client, sur son URI de retour vérifiée.
    Redirect(String),
}

/// Client d'un `client_id` : document de métadonnées (lu ou relu) ou client enregistré.
async fn resolve_client(
    state: &OAuthState,
    client_id: &str,
) -> Result<Option<ClientRecord>, OAuthError> {
    let mut conn = state.pool.acquire().await?;
    let cached = OAuthRepository::client(&mut conn, client_id).await?;
    if !metadata_url_allowed(client_id) {
        return Ok(cached.filter(|c| c.registration == "dynamic"));
    }
    let fresh = cached
        .as_ref()
        .and_then(|c| c.fetched_at)
        .is_some_and(|at| OffsetDateTime::now_utc() - at < METADATA_TTL);
    if fresh {
        return Ok(cached);
    }
    let document = match state.fetcher.fetch(client_id).await {
        Ok(document) => document,
        Err(error) => {
            tracing::warn!(%error, client_id, "client metadata document unavailable");
            // Document momentanément illisible : la version en cache reste valable.
            return Ok(cached);
        }
    };
    let Ok(metadata) = serde_json::from_slice::<ClientMetadata>(&document) else {
        return Ok(None);
    };
    if metadata.client_id.as_deref() != Some(client_id) {
        return Ok(None);
    }
    let Ok(valid) = metadata.validate() else {
        return Ok(None);
    };
    Ok(Some(
        OAuthRepository::save_metadata_client(&mut conn, client_id, &valid).await?,
    ))
}

/// `GET /oauth/authorize` : vérifie la demande et l'enregistre en attente du consentement.
pub async fn authorize(
    state: &OAuthState,
    params: AuthorizeParams,
) -> Result<Authorize, OAuthError> {
    let Some(client_id) = params.client_id.as_deref() else {
        return Ok(Authorize::Refused("missing_client_id"));
    };
    let Some(client) = resolve_client(state, client_id).await? else {
        return Ok(Authorize::Refused("unknown_client"));
    };
    let redirect_uri = match params.redirect_uri.as_deref() {
        Some(uri)
            if client
                .redirect_uris
                .iter()
                .any(|u| redirect_uri_matches(u, uri)) =>
        {
            uri.to_owned()
        }
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        _ => return Ok(Authorize::Refused("invalid_redirect_uri")),
    };

    let state_param = params.state.as_deref().filter(|s| valid_state(s));
    let error = |code: &str, description: &str| {
        let mut query = vec![("error", code), ("error_description", description)];
        if let Some(s) = state_param {
            query.push(("state", s));
        }
        query.push(("iss", state.issuer.as_str()));
        Authorize::Redirect(with_query(&redirect_uri, &query))
    };
    if params.response_type.as_deref() != Some("code") {
        return Ok(error(
            "unsupported_response_type",
            "response_type must be code",
        ));
    }
    if params.state.is_some() && state_param.is_none() {
        return Ok(error("invalid_request", "invalid state"));
    }
    let challenge = params.code_challenge.as_deref().unwrap_or_default();
    if params.code_challenge_method.as_deref() != Some("S256") || !valid_challenge(challenge) {
        return Ok(error("invalid_request", "PKCE with S256 is required"));
    }
    if params
        .resource
        .as_deref()
        .is_some_and(|r| r.trim_end_matches('/') != state.resource())
    {
        return Ok(error("invalid_target", "unknown resource"));
    }
    let scopes = requested_scopes(params.scope.as_deref());
    if scopes.is_empty() {
        return Ok(error("invalid_scope", "no grantable scope requested"));
    }

    let mut conn = state.pool.acquire().await?;
    OAuthRepository::purge(&mut conn, DYNAMIC_CLIENT_TTL).await?;
    let id = Uuid::now_v7();
    let scopes: Vec<String> = scopes.iter().map(|p| p.key().to_owned()).collect();
    OAuthRepository::insert_request(
        &mut conn,
        &NewRequest {
            id,
            client: client.id,
            redirect_uri: &redirect_uri,
            state: state_param,
            code_challenge: challenge,
            scopes: &scopes,
        },
        REQUEST_TTL,
    )
    .await?;
    Ok(Authorize::Consent(id))
}

/// Droit demandé, vu par l'admin qui consent.
#[derive(Debug, Clone)]
pub struct RequestedScope {
    pub permission: Permission,
    /// L'admin le détient : il peut l'accorder.
    pub held: bool,
}

#[derive(Debug, Clone)]
pub struct ConsentView {
    pub request: RequestRecord,
    pub scopes: Vec<RequestedScope>,
}

fn permissions(keys: &[String]) -> Vec<Permission> {
    keys.iter()
        .filter_map(|key| Permission::from_key(key))
        .collect()
}

/// Demande à présenter sur la page de consentement.
pub async fn consent_view(
    state: &OAuthState,
    principal: &Principal,
    id: Uuid,
) -> Result<ConsentView, ConsentError> {
    let mut conn = state.pool.acquire().await?;
    let request = OAuthRepository::pending_request(&mut conn, id, false)
        .await?
        .ok_or(ConsentError::NotFound)?;
    let scopes = permissions(&request.scopes)
        .into_iter()
        .map(|permission| RequestedScope {
            permission,
            held: principal.permissions.contains(permission),
        })
        .collect();
    Ok(ConsentView { request, scopes })
}

/// L'admin accorde la demande, éventuellement réduite à `chosen` : seuls les droits demandés,
/// détenus et accordables sont retenus. Rend l'URI de retour du client, avec le code.
pub async fn approve(
    state: &OAuthState,
    principal: &Principal,
    context: &AuditContext,
    id: Uuid,
    chosen: Option<Vec<String>>,
) -> Result<String, ConsentError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let request = OAuthRepository::pending_request(&mut tx, id, true)
        .await?
        .ok_or(ConsentError::NotFound)?;
    let requested = permissions(&request.scopes);
    let retained: Vec<Permission> = match chosen {
        Some(chosen) => permissions(&chosen)
            .into_iter()
            .filter(|p| requested.contains(p))
            .collect(),
        None => requested,
    }
    .into_iter()
    .filter(|p| principal.permissions.contains(*p) && p.agent_grantable())
    .collect();
    if retained.is_empty() {
        return Err(ConsentError::NoScope);
    }
    let scopes: Vec<String> = retained.iter().map(|p| p.key().to_owned()).collect();

    let grant = Uuid::now_v7();
    let replaced = OAuthRepository::replace_grant(
        &mut tx,
        grant,
        principal.user_id,
        request.client_uuid,
        &scopes,
    )
    .await?;
    let code = opaque::generate(CODE_PREFIX)?;
    OAuthRepository::set_code(&mut tx, id, grant, &opaque::hash(&code), CODE_TTL).await?;
    tx.commit(
        AuditEvent::success(AuditAction::OAuthGrantApproved)
            .target("oauth_grant", grant)
            .details(json!({
                "client": request.client_id,
                "scopes": scopes,
                "replaced": replaced,
            })),
    )
    .await?;

    let mut query = vec![("code", code.as_str())];
    if let Some(s) = request.state.as_deref() {
        query.push(("state", s));
    }
    query.push(("iss", state.issuer.as_str()));
    Ok(with_query(&request.redirect_uri, &query))
}

/// L'admin refuse : la demande est effacée, le client reçoit `access_denied`.
pub async fn deny(
    state: &OAuthState,
    context: &AuditContext,
    id: Uuid,
) -> Result<String, ConsentError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let request = OAuthRepository::pending_request(&mut tx, id, true)
        .await?
        .ok_or(ConsentError::NotFound)?;
    OAuthRepository::delete_request(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::OAuthGrantDenied)
            .details(json!({ "client": request.client_id })),
    )
    .await?;
    let mut query = vec![("error", "access_denied")];
    if let Some(s) = request.state.as_deref() {
        query.push(("state", s));
    }
    query.push(("iss", state.issuer.as_str()));
    Ok(with_query(&request.redirect_uri, &query))
}

/// Paramètres de `POST /oauth/token` (formulaire).
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TokenParams {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub resource: Option<String>,
}

/// Jetons émis (RFC 6749 §5.1).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: String,
}

/// `POST /oauth/token`.
pub async fn token(
    state: &OAuthState,
    context: &AuditContext,
    params: TokenParams,
) -> Result<TokenResponse, OAuthError> {
    if params
        .resource
        .as_deref()
        .is_some_and(|r| r.trim_end_matches('/') != state.resource())
    {
        return Err(OAuthError::InvalidTarget);
    }
    let client_id = params
        .client_id
        .as_deref()
        .ok_or(OAuthError::InvalidClient("client_id is required"))?;
    match params.grant_type.as_deref() {
        Some("authorization_code") => {
            let code = params
                .code
                .as_deref()
                .ok_or(OAuthError::InvalidRequest("code is required"))?;
            let verifier = params
                .code_verifier
                .as_deref()
                .ok_or(OAuthError::InvalidRequest("code_verifier is required"))?;
            exchange_code(
                state,
                context,
                client_id,
                code,
                verifier,
                params.redirect_uri.as_deref(),
            )
            .await
        }
        Some("refresh_token") => {
            let refresh = params
                .refresh_token
                .as_deref()
                .ok_or(OAuthError::InvalidRequest("refresh_token is required"))?;
            refresh_token(state, context, client_id, refresh).await
        }
        _ => Err(OAuthError::UnsupportedGrantType),
    }
}

async fn exchange_code(
    state: &OAuthState,
    context: &AuditContext,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: Option<&str>,
) -> Result<TokenResponse, OAuthError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    // Le code est effacé dès sa présentation : un second essai échoue, même après une erreur.
    let redeemed = OAuthRepository::redeem_code(&mut tx, &opaque::hash(code)).await?;
    let valid = redeemed.filter(|r| {
        !r.expired
            && r.client_id == client_id
            && redirect_uri.is_none_or(|uri| uri == r.redirect_uri)
            && pkce_matches(verifier, &r.code_challenge)
    });
    let Some(redeemed) = valid else {
        tx.commit(
            AuditEvent::failure(AuditAction::OAuthTokenIssued)
                .details(json!({ "client": client_id, "reason": "invalid_grant" })),
        )
        .await?;
        return Err(OAuthError::InvalidGrant("invalid or expired code"));
    };
    let (user_id, response) = issue(&mut tx, redeemed.grant_id).await?;
    tx.set_actor(Actor::agent(user_id));
    tx.commit(
        AuditEvent::success(AuditAction::OAuthTokenIssued)
            .target("oauth_grant", redeemed.grant_id)
            .details(json!({ "client": client_id })),
    )
    .await?;
    Ok(response)
}

/// Rotation : l'ancien jeton est consommé. Un jeton déjà consommé signe un vol : l'accord est
/// révoqué, ses jetons avec lui.
async fn refresh_token(
    state: &OAuthState,
    context: &AuditContext,
    client_id: &str,
    refresh: &str,
) -> Result<TokenResponse, OAuthError> {
    let invalid = || OAuthError::InvalidGrant("invalid or expired refresh token");
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let hash = opaque::hash(refresh);
    let Some(record) = OAuthRepository::refresh_token(&mut tx, &hash).await? else {
        return Err(invalid());
    };
    if record.client_id != client_id {
        return Err(invalid());
    }
    tx.set_actor(Actor::agent(record.user_id));
    if record.consumed {
        OAuthRepository::delete_grant(&mut tx, record.grant_id, None).await?;
        tx.commit(
            AuditEvent::failure(AuditAction::OAuthRefreshReused)
                .target("oauth_grant", record.grant_id)
                .details(json!({ "client": client_id })),
        )
        .await?;
        return Err(invalid());
    }
    if record.expired {
        return Err(invalid());
    }
    OAuthRepository::consume(&mut tx, &hash).await?;
    let (_, response) = issue(&mut tx, record.grant_id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::OAuthTokenRefreshed)
            .target("oauth_grant", record.grant_id)
            .details(json!({ "client": client_id })),
    )
    .await?;
    Ok(response)
}

/// Nouveaux jetons d'accès et de rafraîchissement pour un accord. Rend l'admin de l'accord.
async fn issue(tx: &mut AuditedTx, grant: Uuid) -> Result<(Uuid, TokenResponse), OAuthError> {
    let (user_id, scopes) = OAuthRepository::grant_owner(tx, grant)
        .await?
        .ok_or(OAuthError::InvalidGrant("grant revoked"))?;
    OAuthRepository::purge_tokens(tx, grant).await?;
    let access_token = opaque::generate(ACCESS_PREFIX)?;
    let refresh_token = opaque::generate(REFRESH_PREFIX)?;
    OAuthRepository::insert_token(
        tx,
        &opaque::hash(&access_token),
        grant,
        TokenKind::Access,
        ACCESS_TTL,
    )
    .await?;
    OAuthRepository::insert_token(
        tx,
        &opaque::hash(&refresh_token),
        grant,
        TokenKind::Refresh,
        REFRESH_TTL,
    )
    .await?;
    Ok((
        user_id,
        TokenResponse {
            access_token,
            token_type: "Bearer",
            expires_in: ACCESS_TTL.as_secs(),
            refresh_token,
            scope: scope_string(&permissions(&scopes)),
        },
    ))
}

/// `POST /oauth/revoke` (RFC 7009) : un jeton inconnu n'est pas une erreur. Révoquer un jeton
/// de rafraîchissement révoque tout l'accord.
pub async fn revoke(
    state: &OAuthState,
    context: &AuditContext,
    token: &str,
    client_id: Option<&str>,
) -> Result<(), OAuthError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let hash = opaque::hash(token);
    let Some((grant, client, refresh)) = OAuthRepository::delete_token(&mut tx, &hash).await?
    else {
        return Ok(());
    };
    if client_id.is_some_and(|c| c != client) {
        // Jeton d'un autre client : laissé en place.
        return Ok(());
    }
    if !refresh {
        tx.commit(
            AuditEvent::success(AuditAction::OAuthGrantRevoked)
                .target("oauth_grant", grant)
                .details(json!({ "client": client, "by": "client", "token": "access" })),
        )
        .await?;
        return Ok(());
    }
    if let Some((user_id, _)) = OAuthRepository::grant_owner(&mut tx, grant).await? {
        tx.set_actor(Actor::agent(user_id));
    }
    OAuthRepository::delete_grant(&mut tx, grant, None).await?;
    tx.commit(
        AuditEvent::success(AuditAction::OAuthGrantRevoked)
            .target("oauth_grant", grant)
            .details(json!({ "client": client, "by": "client" })),
    )
    .await?;
    Ok(())
}

/// Client enregistré dynamiquement (RFC 7591).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Registered {
    pub client_id: String,
    pub client_id_issued_at: i64,
    pub client_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    pub redirect_uris: Vec<String>,
    pub token_endpoint_auth_method: &'static str,
    pub grant_types: [&'static str; 2],
    pub response_types: [&'static str; 1],
}

/// `POST /oauth/register` : clients publics seulement, débit borné par adresse.
pub async fn register(
    state: &OAuthState,
    context: &AuditContext,
    metadata: ClientMetadata,
) -> Result<Registered, OAuthError> {
    let ip = context.ip.as_deref().and_then(|ip| ip.parse().ok());
    if !state.allow_registration(ip) {
        return Err(OAuthError::SlowDown);
    }
    let client = metadata
        .validate()
        .map_err(OAuthError::InvalidClientMetadata)?;
    let id = Uuid::now_v7();
    let client_id = format!("kuro-client-{}", id.simple());
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    OAuthRepository::purge(&mut tx, DYNAMIC_CLIENT_TTL).await?;
    OAuthRepository::insert_dynamic_client(&mut tx, id, &client_id, &client).await?;
    tx.commit(
        AuditEvent::success(AuditAction::OAuthClientRegistered)
            .target("oauth_client", id)
            .details(json!({ "client": client_id })),
    )
    .await?;
    Ok(Registered {
        client_id,
        client_id_issued_at: OffsetDateTime::now_utc().unix_timestamp(),
        client_name: client.name,
        client_uri: client.uri,
        redirect_uris: client.redirect_uris,
        token_endpoint_auth_method: "none",
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
    })
}

/// Accords de l'admin connecté (« Applications connectées »).
pub async fn grants(
    state: &OAuthState,
    principal: &Principal,
) -> Result<Vec<GrantRecord>, ConsentError> {
    let mut conn = state.pool.acquire().await?;
    Ok(OAuthRepository::grants_of(&mut conn, principal.user_id).await?)
}

/// L'admin révoque un de ses accords.
pub async fn revoke_grant(
    state: &OAuthState,
    principal: &Principal,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), ConsentError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let client = OAuthRepository::delete_grant(&mut tx, id, Some(principal.user_id))
        .await?
        .ok_or(ConsentError::NotFound)?;
    tx.commit(
        AuditEvent::success(AuditAction::OAuthGrantRevoked)
            .target("oauth_grant", id)
            .details(json!({ "client": client, "by": "admin" })),
    )
    .await?;
    Ok(())
}

/// Agent authentifié par son jeton d'accès.
#[derive(Debug, Clone)]
pub struct AgentPrincipal {
    /// Droits effectifs : accordés ∩ droits actuels de l'admin.
    pub principal: Principal,
    pub grant_id: Uuid,
    pub client_id: String,
}

/// Valide un jeton d'accès d'agent et recalcule ses droits : un admin désactivé, supprimé ou
/// privé d'un droit le retire aussitôt à l'agent, comme la coupure du MCP dans les paramètres.
pub async fn authenticate(
    pool: &PgPool,
    access_token: &str,
) -> Result<Option<AgentPrincipal>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    if !SettingsRepository::mcp_enabled(&mut conn).await? {
        return Ok(None);
    }
    let Some(access) =
        OAuthRepository::access_token(&mut conn, &opaque::hash(access_token)).await?
    else {
        return Ok(None);
    };
    let active = UserRepository::get(&mut conn, access.user_id)
        .await?
        .is_some_and(|user| !user.disabled);
    if !active {
        return Ok(None);
    }
    let held =
        PermissionSet::from_keys(UserRepository::permission_keys(&mut conn, access.user_id).await?);
    let effective = permissions(&access.scopes)
        .into_iter()
        .filter(|p| held.contains(*p) && p.agent_grantable());
    Ok(Some(AgentPrincipal {
        principal: Principal {
            user_id: access.user_id,
            session_id: access.grant_id,
            permissions: PermissionSet::from_permissions(effective),
        },
        grant_id: access.grant_id,
        client_id: access.client_id,
    }))
}
