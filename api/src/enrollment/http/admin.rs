//! Espace admin : création d'un nœud et réémission de son token (`/api/admin/v1/nodes…`).
//! Mêmes cas d'usage que `kuro-api node add` et `kuro-api node token`.

use std::time::Duration;

use crate::http::extract::Json;
use crate::http::extract::Path;
use axum::extract::{FromRef, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::enrollment::application::{AddNodeError, NodeTicket, add_node, reissue_token};
use crate::http::error::{ApiError, ErrorCode, FieldError, Problem, error_response};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::nodes::application::PlacementInput;
use crate::nodes::http::NodeDto;
use crate::nodes::http::nodes::CoordinatesDto;

/// Paramètres d'enrôlement communiqués à l'opérateur avec le token.
#[derive(Debug, Clone)]
pub struct EnrollmentSettings {
    pub token_ttl: Duration,
    /// Empreinte de la CA, à passer à `kuro-daemonctl enroll --ca-fingerprint`.
    pub ca_fingerprint: String,
    /// Port du listener daemon.
    pub daemon_port: u16,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateNodeRequest {
    /// Identifiant technique (`[a-z0-9-]`), inscrit dans le certificat : non modifiable ensuite.
    #[schema(example = "par-01")]
    pub name: String,
    pub display_name: Option<String>,
    pub region_id: Option<Uuid>,
    pub location: Option<String>,
    pub coordinates: Option<CoordinatesDto>,
}

/// À transmettre à l'opérateur de l'hôte : le token n'est montré qu'une fois.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnrollmentTicketDto {
    #[schema(example = "kuro_enr_…")]
    pub token: String,
    pub expires_in: u64,
    pub ca_fingerprint: String,
    pub daemon_port: u16,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NodeTicketDto {
    pub node: NodeDto,
    pub enrollment: EnrollmentTicketDto,
}

/// Crée un nœud `pending` et son token d'enrôlement. Permission : `nodes.create`.
#[utoipa::path(post, path = "/api/admin/v1/nodes", tag = "admin-nodes", security(("bearer" = [])),
    request_body = CreateNodeRequest,
    responses((status = 201, body = NodeTicketDto), (status = 403, body = Problem),
              (status = 409, body = Problem), (status = 422, body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(settings): State<EnrollmentSettings>,
    session: AdminSession,
    Json(request): Json<CreateNodeRequest>,
) -> Result<(StatusCode, Json<NodeTicketDto>), ApiError> {
    session.require(&iam, Permission::NodesCreate).await?;
    let placement = PlacementInput {
        region_id: request.region_id,
        display_name: request.display_name,
        location: request.location,
        coordinates: request.coordinates.map(|c| (c.latitude, c.longitude)),
    };
    let ticket = add_node(
        &pool,
        &session.context,
        &request.name,
        placement,
        settings.token_ttl,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(ticket_dto(ticket, &settings))))
}

/// Nouveau token pour un nœud en attente ; les précédents sont invalidés.
/// Permission : `nodes.enrollment`.
#[utoipa::path(post, path = "/api/admin/v1/nodes/{id}/enrollment-token", tag = "admin-nodes",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = NodeTicketDto), (status = 403, body = Problem),
              (status = 404, body = Problem),
              (status = 409, description = "Nœud déjà enrôlé", body = Problem)))]
pub async fn reissue(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(settings): State<EnrollmentSettings>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeTicketDto>, ApiError> {
    session
        .require_elevated(&iam, Permission::NodesEnrollment)
        .await?;
    let ticket = reissue_token(&pool, &session.context, id, settings.token_ttl).await?;
    Ok(Json(ticket_dto(ticket, &settings)))
}

pub fn routes<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    PgPool: FromRef<S>,
    EnrollmentSettings: FromRef<S>,
{
    use axum::routing::post;
    axum::Router::new()
        .route("/nodes", post(create))
        .route("/nodes/{id}/enrollment-token", post(reissue))
}

fn ticket_dto(ticket: NodeTicket, settings: &EnrollmentSettings) -> NodeTicketDto {
    NodeTicketDto {
        node: ticket.node.into(),
        enrollment: EnrollmentTicketDto {
            token: ticket.token.expose().to_owned(),
            expires_in: ticket.expires_in.as_secs(),
            ca_fingerprint: settings.ca_fingerprint.clone(),
            daemon_port: settings.daemon_port,
        },
    }
}

impl IntoResponse for AddNodeError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidName(_) => {
                Problem::invalid(FieldError::new("name", ErrorCode::InvalidNodeName))
                    .into_response()
            }
            Self::AlreadyExists => error_response(StatusCode::CONFLICT, ErrorCode::NodeNameTaken),
            Self::Placement(error) => error.into_response(),
            Self::Random(error) => {
                tracing::error!(%error, "node creation failed");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
            }
            Self::Database(error) => {
                tracing::error!(%error, "node creation failed");
                error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
            }
        }
    }
}

impl From<AddNodeError> for ApiError {
    fn from(error: AddNodeError) -> Self {
        Self::from(error.into_response())
    }
}
