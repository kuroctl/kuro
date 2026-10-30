use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::servers::application::ServersError;

impl IntoResponse for ServersError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => error_response(
                StatusCode::NOT_FOUND,
                crate::error_code::ErrorCode::NotFound,
            ),
            Self::Missing(code) => error_response(StatusCode::NOT_FOUND, code),
            Self::Invalid(field) => Problem::invalid(field).into_response(),
            Self::InvalidSettings(fields) => Problem::invalid_fields(fields).into_response(),
            Self::Unprocessable(code) => error_response(StatusCode::UNPROCESSABLE_ENTITY, code),
            Self::Conflict(code) => error_response(StatusCode::CONFLICT, code),
            Self::Forbidden(permission) => Problem::new(
                StatusCode::FORBIDDEN,
                crate::error_code::ErrorCode::Forbidden,
            )
            .param("permission", permission.key())
            .into_response(),
            Self::Daemon { code, message } => {
                // Détail technique dans les logs ; le client reçoit le code stable.
                tracing::warn!(?code, %message, "daemon refused a command");
                error_response(StatusCode::BAD_GATEWAY, code)
            }
            Self::Internal(error) => internal_error(&error, "servers request failed"),
        }
    }
}

impl From<ServersError> for ApiError {
    fn from(error: ServersError) -> Self {
        Self::from(error.into_response())
    }
}
