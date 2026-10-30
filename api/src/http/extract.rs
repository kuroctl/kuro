//! Extracteurs axum dont les rejets suivent le contrat d'erreur (problem+json, `code`).
//!
//! À utiliser à la place de `axum::Json`, `axum::extract::Path` et `axum::extract::Query`.
//! `Json` sert aussi pour les réponses.

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::{ErrorCode, Problem};

/// Corps JSON. Rejet : `invalid_body` (415, 400 ou 422 selon la cause), cause dans `params.reason`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<S, T> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        axum::Json::<T>::from_request(request, state)
            .await
            .map(|axum::Json(value)| Self(value))
            .map_err(|rejection: JsonRejection| {
                Problem::new(rejection.status(), ErrorCode::InvalidBody)
                    .param("reason", rejection.body_text())
                    .into_response()
            })
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// Paramètres de chemin. Rejet : `invalid_parameter`.
#[derive(Debug, Clone, Copy)]
pub struct Path<T>(pub T);

impl<S, T> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Path(value)| Self(value))
            .map_err(|rejection: PathRejection| {
                Problem::new(rejection.status(), ErrorCode::InvalidParameter)
                    .param("reason", rejection.body_text())
                    .into_response()
            })
    }
}

/// Paramètres de requête. Rejet : `invalid_parameter`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<S, T> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        axum::extract::Query::<T>::from_request_parts(parts, state)
            .await
            .map(|axum::extract::Query(value)| Self(value))
            .map_err(|rejection: QueryRejection| {
                Problem::new(rejection.status(), ErrorCode::InvalidParameter)
                    .param("reason", rejection.body_text())
                    .into_response()
            })
    }
}

/// Route inconnue.
pub async fn not_found() -> Response {
    Problem::new(axum::http::StatusCode::NOT_FOUND, ErrorCode::NotFound).into_response()
}

/// Méthode non prise en charge sur une route existante.
pub async fn method_not_allowed() -> Response {
    Problem::new(
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        ErrorCode::NotFound,
    )
    .into_response()
}
