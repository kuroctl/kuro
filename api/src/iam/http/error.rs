use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::http::error::{
    ApiError, ErrorCode, FieldError, Problem, error_response, internal_error,
};
use crate::iam::application::IamError;

impl IntoResponse for IamError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidCredentials => {
                error_response(StatusCode::UNAUTHORIZED, ErrorCode::InvalidCredentials)
            }
            Self::InvalidChallenge => {
                error_response(StatusCode::UNAUTHORIZED, ErrorCode::InvalidChallenge)
            }
            Self::InvalidCode => error_response(StatusCode::UNAUTHORIZED, ErrorCode::InvalidCode),
            Self::InvalidSession => {
                error_response(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized)
            }
            Self::TotpAlreadyEnabled => {
                error_response(StatusCode::CONFLICT, ErrorCode::TotpAlreadyEnabled)
            }
            Self::TotpSetupNotStarted => {
                error_response(StatusCode::CONFLICT, ErrorCode::TotpSetupNotStarted)
            }
            Self::NotFound => error_response(StatusCode::NOT_FOUND, ErrorCode::NotFound),
            Self::Forbidden(code) => error_response(StatusCode::FORBIDDEN, code),
            Self::Conflict(code) => error_response(StatusCode::CONFLICT, code),
            Self::Invalid(field) => Problem::invalid(field).into_response(),
            Self::Unprocessable(code) => error_response(StatusCode::UNPROCESSABLE_ENTITY, code),
            Self::UnknownPermission(key) => Problem::invalid(
                FieldError::new("permissions", ErrorCode::UnknownPermission)
                    .param("permission", key),
            )
            .into_response(),
            Self::PermissionNotAssignable(key) => Problem::invalid(
                FieldError::new("permissions", ErrorCode::PermissionNotAssignable)
                    .param("permission", key),
            )
            .into_response(),
            Self::Internal(error) => internal_error(&error, "iam request failed"),
        }
    }
}

impl From<IamError> for ApiError {
    fn from(error: IamError) -> Self {
        Self::from(error.into_response())
    }
}
