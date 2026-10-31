//! `/auth/*` : connexion, mot de passe temporaire à remplacer, TOTP, rafraîchissement, déconnexion.
//!
//! Le jeton de rafraîchissement ne circule que dans un cookie `__Host-` (`HttpOnly`, `Secure`,
//! `SameSite=Strict`, chemin `/`, sans `Domain`) ; le jeton d'accès est renvoyé dans le corps.
//! Le préfixe `__Host-` interdit à un autre sous-domaine (un site web hébergé, ADR-0038) de poser
//! un cookie homonyme ; en échange, le cookie part avec toutes les requêtes vers l'API, où seules
//! ces routes le lisent. Toute route qui pose ou lit le cookie vérifie aussi l'en-tête `Origin`
//! ([`BrowserOrigin`]) : un site « même site » ne peut ni ouvrir, ni faire tourner, ni fermer la
//! session de l'admin.

use crate::http::extract::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::AdminSession;
use crate::http::context::RequestContext;
use crate::http::error::Problem;
use crate::http::origin::BrowserOrigin;
use crate::iam::application::{self, IamError, IamState, LoginOutcome, Refreshed, SessionTokens};

pub(super) const REFRESH_COOKIE: &str = "__Host-kuro_admin_refresh";
/// Imposé par le préfixe `__Host-`.
const REFRESH_COOKIE_PATH: &str = "/";

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    #[schema(example = "admin@example.com")]
    pub email: String,
    pub password: String,
}

/// Jamais le mot de passe, même dans un journal de débogage.
impl std::fmt::Debug for LoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginRequest")
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginResponse {
    /// Session ouverte (cookie de rafraîchissement posé).
    Authenticated {
        access_token: String,
        token_type: &'static str,
        expires_in: u64,
    },
    /// Mot de passe choisi par un autre admin : le remplacer via `/auth/password`.
    PasswordChangeRequired { challenge: String, expires_in: u64 },
    /// Envoyer le code TOTP à `/auth/totp/verify`.
    TotpRequired { challenge: String, expires_in: u64 },
    /// TOTP obligatoire : le configurer via `/auth/totp/setup` puis `/auth/totp/confirm`.
    TotpSetupRequired { challenge: String, expires_in: u64 },
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ChallengeRequest {
    pub challenge: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TotpCodeRequest {
    pub challenge: String,
    #[schema(example = "123456")]
    pub code: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RequiredPasswordRequest {
    pub challenge: String,
    /// 12 caractères minimum, différent du mot de passe temporaire.
    pub new_password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TotpSetupResponse {
    /// Secret en base32, pour une saisie manuelle.
    pub secret: String,
    /// À afficher en QR code.
    pub otpauth_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccessTokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
}

/// Première étape de connexion : e-mail et mot de passe.
#[utoipa::path(post, path = "/api/admin/v1/auth/login", tag = "admin-auth",
    request_body = LoginRequest,
    responses(
        (status = 200, body = LoginResponse),
        (status = 401, description = "Identifiants invalides (compte inconnu, désactivé ou verrouillé inclus)", body = Problem),
        (status = 403, description = "Origine de la page non autorisée", body = Problem),
    ))]
pub async fn login(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    jar: CookieJar,
    Json(request): Json<LoginRequest>,
) -> Result<Response, IamError> {
    let outcome = application::login(&state, &context, &request.email, &request.password).await?;
    Ok(login_response(&state, jar, outcome))
}

/// Remplace le mot de passe choisi par un autre admin, puis poursuit la connexion : même réponse
/// que `/auth/login` (TOTP attendu, à configurer, ou session ouverte).
#[utoipa::path(post, path = "/api/admin/v1/auth/password", tag = "admin-auth",
    request_body = RequiredPasswordRequest,
    responses(
        (status = 200, body = LoginResponse),
        (status = 401, description = "Challenge invalide ou expiré", body = Problem),
        (status = 403, description = "Origine de la page non autorisée", body = Problem),
        (status = 422, description = "Mot de passe trop faible ou inchangé", body = Problem),
    ))]
pub async fn change_required_password(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    jar: CookieJar,
    Json(request): Json<RequiredPasswordRequest>,
) -> Result<Response, IamError> {
    let outcome = application::change_required_password(
        &state,
        &context,
        &request.challenge,
        &request.new_password,
    )
    .await?;
    Ok(login_response(&state, jar, outcome))
}

/// Invitation acceptée : le jeton du lien et le mot de passe choisi.
#[derive(Debug, Deserialize, ToSchema)]
pub struct InvitationRequest {
    pub token: String,
    /// 12 caractères minimum.
    pub password: String,
}

/// Accepte une invitation (ADR-0046) : pose le mot de passe, puis répond comme `login`
/// (session, ou TOTP à configurer). Jeton inconnu, expiré ou déjà utilisé : `403`.
#[utoipa::path(post, path = "/api/admin/v1/auth/invitation", tag = "admin-auth",
    request_body = InvitationRequest,
    responses(
        (status = 200, body = LoginResponse),
        (status = 403, description = "Jeton invalide ou origine de la page non autorisée", body = Problem),
        (status = 422, body = Problem),
    ))]
pub async fn accept_invitation(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    jar: CookieJar,
    Json(request): Json<InvitationRequest>,
) -> Result<Response, IamError> {
    let outcome = crate::iam::application::invitations::accept(
        &state,
        &context,
        &request.token,
        &request.password,
    )
    .await?;
    Ok(login_response(&state, jar, outcome))
}

fn login_response(state: &IamState, jar: CookieJar, outcome: LoginOutcome) -> Response {
    match outcome {
        LoginOutcome::Authenticated(tokens) => authenticated(state, jar, tokens, true),
        LoginOutcome::PasswordChangeRequired(challenge) => {
            Json(LoginResponse::PasswordChangeRequired {
                challenge: challenge.token,
                expires_in: challenge.expires_in,
            })
            .into_response()
        }
        LoginOutcome::TotpRequired(challenge) => Json(LoginResponse::TotpRequired {
            challenge: challenge.token,
            expires_in: challenge.expires_in,
        })
        .into_response(),
        LoginOutcome::TotpSetupRequired(challenge) => Json(LoginResponse::TotpSetupRequired {
            challenge: challenge.token,
            expires_in: challenge.expires_in,
        })
        .into_response(),
    }
}

/// Code TOTP après un mot de passe valide.
#[utoipa::path(post, path = "/api/admin/v1/auth/totp/verify", tag = "admin-auth",
    request_body = TotpCodeRequest,
    responses(
        (status = 200, body = AccessTokenResponse),
        (status = 401, description = "Challenge ou code invalide", body = Problem),
        (status = 403, description = "Origine de la page non autorisée", body = Problem),
    ))]
pub async fn verify_totp(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    jar: CookieJar,
    Json(request): Json<TotpCodeRequest>,
) -> Result<Response, IamError> {
    let tokens =
        application::verify_totp(&state, &context, &request.challenge, &request.code).await?;
    Ok(authenticated(&state, jar, tokens, false))
}

/// Démarre la configuration du TOTP (compte sans TOTP, challenge de configuration).
#[utoipa::path(post, path = "/api/admin/v1/auth/totp/setup", tag = "admin-auth",
    request_body = ChallengeRequest,
    responses(
        (status = 200, body = TotpSetupResponse),
        (status = 401, body = Problem),
        (status = 409, description = "TOTP déjà configuré", body = Problem),
    ))]
pub async fn start_totp_setup(
    State(state): State<IamState>,
    RequestContext(context): RequestContext,
    Json(request): Json<ChallengeRequest>,
) -> Result<Json<TotpSetupResponse>, IamError> {
    let setup = application::start_totp_setup(&state, &context, &request.challenge).await?;
    Ok(Json(TotpSetupResponse {
        secret: setup.secret,
        otpauth_uri: setup.otpauth_uri,
    }))
}

/// Confirme le TOTP avec un premier code et ouvre la session.
#[utoipa::path(post, path = "/api/admin/v1/auth/totp/confirm", tag = "admin-auth",
    request_body = TotpCodeRequest,
    responses(
        (status = 200, body = AccessTokenResponse),
        (status = 401, body = Problem),
        (status = 403, description = "Origine de la page non autorisée", body = Problem),
        (status = 409, body = Problem),
    ))]
pub async fn confirm_totp(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    jar: CookieJar,
    Json(request): Json<TotpCodeRequest>,
) -> Result<Response, IamError> {
    let tokens =
        application::confirm_totp(&state, &context, &request.challenge, &request.code).await?;
    Ok(authenticated(&state, jar, tokens, false))
}

/// Nouveau jeton d'accès à partir du cookie de rafraîchissement (rotation). Un cookie remplacé
/// à l'instant (10 s) par une autre requête donne le même successeur que celle-ci.
#[utoipa::path(post, path = "/api/admin/v1/auth/refresh", tag = "admin-auth",
    responses(
        (status = 200, body = AccessTokenResponse),
        (status = 401, description = "Cookie absent, expiré, révoqué ou rejoué", body = Problem),
        (status = 403, description = "Origine de la page non autorisée", body = Problem),
    ))]
pub async fn refresh(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    jar: CookieJar,
) -> Result<Response, IamError> {
    let token = jar
        .get(REFRESH_COOKIE)
        .map(|cookie| cookie.value().to_owned())
        .ok_or(IamError::InvalidSession)?;
    match application::refresh(&state, &context, &token).await {
        Ok(Refreshed::Rotated(tokens)) => Ok(authenticated(&state, jar, tokens, false)),
        Ok(Refreshed::Grace(tokens)) => Ok(authenticated(&state, jar, tokens, false)),
        Ok(Refreshed::GraceAccessOnly(token)) => Ok(Json(AccessTokenResponse {
            access_token: token.access_token,
            token_type: "Bearer",
            expires_in: token.expires_in,
        })
        .into_response()),
        // Un cookie invalide est effacé pour ne pas être renvoyé indéfiniment.
        Err(error) => Ok((clear_cookie(jar), error).into_response()),
    }
}

/// Ferme la session courante (tous ses jetons de rafraîchissement).
#[utoipa::path(post, path = "/api/admin/v1/auth/logout", tag = "admin-auth",
    security(("bearer" = [])),
    responses((status = 204), (status = 401, body = Problem),
        (status = 403, description = "Origine de la page non autorisée", body = Problem)))]
pub async fn logout(
    State(state): State<IamState>,
    _origin: BrowserOrigin,
    session: AdminSession,
    jar: CookieJar,
) -> Result<Response, IamError> {
    application::logout(&state, &session.context, &session.principal).await?;
    Ok((clear_cookie(jar), StatusCode::NO_CONTENT).into_response())
}

fn authenticated(state: &IamState, jar: CookieJar, tokens: SessionTokens, login: bool) -> Response {
    let cookie = Cookie::build((REFRESH_COOKIE, tokens.refresh_token))
        .http_only(true)
        // Toujours `Secure` (exigé par `__Host-`), même en dev : les navigateurs l'acceptent sur
        // `http://localhost`.
        .secure(true)
        .same_site(SameSite::Strict)
        .path(REFRESH_COOKIE_PATH)
        .max_age(time::Duration::seconds(
            i64::try_from(state.settings.refresh_ttl.as_secs()).unwrap_or(i64::MAX),
        ))
        .build();
    let jar = jar.add(cookie);
    if login {
        (
            jar,
            Json(LoginResponse::Authenticated {
                access_token: tokens.access_token,
                token_type: "Bearer",
                expires_in: tokens.expires_in,
            }),
        )
            .into_response()
    } else {
        (
            jar,
            Json(AccessTokenResponse {
                access_token: tokens.access_token,
                token_type: "Bearer",
                expires_in: tokens.expires_in,
            }),
        )
            .into_response()
    }
}

fn clear_cookie(jar: CookieJar) -> CookieJar {
    jar.remove(
        Cookie::build((REFRESH_COOKIE, ""))
            .path(REFRESH_COOKIE_PATH)
            .build(),
    )
}
