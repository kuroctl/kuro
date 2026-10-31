//! `/api/setup` (ADR-0045) : état « à configurer » (public) et création du premier compte avec
//! le jeton de mise en route. Sans authentification : débit limité par adresse, `Origin`
//! vérifié comme pour la connexion.

use axum::Router;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use super::application::{SetupError, SetupRequest, SetupState};
use crate::error_code::ErrorCode;
use crate::http::context::RequestContext;
use crate::http::error::{Problem, error_response, internal_error};
use crate::http::extract::Json;
use crate::http::origin::BrowserOrigin;
use crate::http::rate_limit::limit_unauthenticated;

#[derive(OpenApi)]
#[openapi(
    paths(status, run),
    components(schemas(SetupStatusDto, SetupRequestDto, SetupDoneDto)),
    tags((name = "setup", description = "Première mise en route (ADR-0045)"))
)]
pub struct SetupApi;

/// Routes montées sous `/api`, hors des espaces admin et client.
pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    SetupState: FromRef<S>,
    // Vérification d'`Origin` et contexte d'audit, comme sur la connexion (ADR-0038).
    crate::http::policy::HttpPolicy: FromRef<S>,
    sqlx::PgPool: FromRef<S>,
{
    Router::new()
        .route("/setup", get(status).post(run))
        .route_layer(from_fn(limit_unauthenticated))
}

impl IntoResponse for SetupError {
    fn into_response(self) -> Response {
        match self {
            Self::Done => error_response(StatusCode::CONFLICT, ErrorCode::SetupDone),
            Self::InvalidToken => error_response(StatusCode::FORBIDDEN, ErrorCode::InvalidToken),
            Self::TokenUnavailable(error) => {
                tracing::error!(%error, "setup token unavailable: use `kuro-api setup`");
                error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::SetupTokenUnavailable,
                )
            }
            Self::Invalid(field) => Problem::invalid_fields(vec![field]).into_response(),
            Self::Internal(error) => internal_error(&error, "setup failed"),
        }
    }
}

/// L'API attend-elle son premier compte, et le nom du projet.
#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusDto {
    /// Vrai tant qu'aucun compte admin n'existe : le panel affiche l'assistant.
    pub needed: bool,
    /// Nom du projet, absent tant qu'il n'est pas réglé (« Kuro »).
    pub project_name: Option<String>,
}

/// Premier compte, super admin, et nom du projet.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRequestDto {
    /// Jeton de mise en route lu sur l'hôte (`/var/lib/kuro/setup-token`).
    pub token: String,
    pub email: String,
    pub display_name: String,
    /// 12 caractères au moins.
    pub password: String,
    /// 1 à 80 caractères ; absent : « Kuro ».
    #[serde(default)]
    pub project_name: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupDoneDto {
    pub user_id: Uuid,
    pub email: String,
}

/// État de la mise en route. Public.
#[utoipa::path(get, path = "/api/setup", tag = "setup",
    responses((status = 200, body = SetupStatusDto)))]
pub async fn status(State(setup): State<SetupState>) -> Result<Json<SetupStatusDto>, SetupError> {
    let status = setup.status().await?;
    Ok(Json(SetupStatusDto {
        needed: status.needed,
        project_name: status.project_name,
    }))
}

/// Crée le premier compte (super admin) et le nom du projet, une seule fois. Le jeton de mise en
/// route est exigé ; la connexion se fait ensuite par `/auth/login`.
#[utoipa::path(post, path = "/api/setup", tag = "setup", request_body = SetupRequestDto,
    responses((status = 201, body = SetupDoneDto),
        (status = 403, description = "Jeton invalide", body = Problem),
        (status = 409, description = "Un compte existe déjà", body = Problem),
        (status = 422, body = Problem),
        (status = 503, description = "Jeton indisponible sur l'hôte : passer par `kuro-api setup`", body = Problem)))]
pub async fn run(
    State(setup): State<SetupState>,
    _origin: BrowserOrigin,
    RequestContext(context): RequestContext,
    Json(request): Json<SetupRequestDto>,
) -> Result<(StatusCode, Json<SetupDoneDto>), SetupError> {
    let user = setup
        .run(
            Some(&request.token),
            &context,
            SetupRequest {
                email: request.email,
                display_name: request.display_name,
                password: request.password,
                project_name: request.project_name,
            },
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(SetupDoneDto {
            user_id: user.id,
            email: user.email,
        }),
    ))
}
