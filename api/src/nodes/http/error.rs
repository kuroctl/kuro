use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::http::error::{
    ApiError, ErrorCode, FieldError, Problem, error_response, internal_error,
};
use crate::nodes::application::NodesError;

impl IntoResponse for NodesError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => error_response(StatusCode::NOT_FOUND, ErrorCode::NotFound),
            Self::UnknownRegion => {
                Problem::invalid(FieldError::new("region_id", ErrorCode::UnknownRegion))
                    .into_response()
            }
            Self::Invalid(field) => Problem::invalid(field).into_response(),
            Self::Unprocessable(code) => error_response(StatusCode::UNPROCESSABLE_ENTITY, code),
            Self::Conflict(code) => error_response(StatusCode::CONFLICT, code),
            Self::Internal(error) => internal_error(&error, "nodes request failed"),
        }
    }
}

impl From<NodesError> for ApiError {
    fn from(error: NodesError) -> Self {
        Self::from(error.into_response())
    }
}
