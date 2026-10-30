//! Erreurs HTTP au format RFC 9457 (`application/problem+json`), étendu de :
//! - `code` : code stable du catalogue [`ErrorCode`], clé de traduction côté client ;
//! - `params` : valeurs à insérer dans le message traduit ;
//! - `fields` : erreurs par champ, pour les formulaires.
//!
//! L'identifiant de requête est dans l'en-tête `x-request-id` de chaque réponse.

use axum::Json;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Map, Value};
use std::borrow::Cow;
use utoipa::ToSchema;

pub use crate::error_code::{ErrorCode, FieldError};

const PROBLEM_JSON: HeaderValue = HeaderValue::from_static("application/problem+json");

#[derive(Debug, Serialize, ToSchema)]
pub struct Problem {
    /// Toujours `about:blank` : le `code` porte la sémantique.
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub status: u16,
    /// Titre anglais de repli ; le client traduit `code`.
    pub title: &'static str,
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "Map::is_empty")]
    #[schema(value_type = Object)]
    pub params: Map<String, Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldProblem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FieldProblem {
    #[schema(value_type = String)]
    pub field: Cow<'static, str>,
    pub code: ErrorCode,
    #[serde(skip_serializing_if = "Map::is_empty")]
    #[schema(value_type = Object)]
    pub params: Map<String, Value>,
}

impl Problem {
    pub fn new(status: StatusCode, code: ErrorCode) -> Self {
        Self {
            kind: "about:blank",
            status: status.as_u16(),
            title: code.title(),
            code,
            params: Map::new(),
            fields: Vec::new(),
        }
    }

    pub fn param(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }

    /// Erreur de validation d'un champ : le code principal est celui du champ.
    /// Plusieurs champs invalides : `validation_failed`, chaque erreur sous son champ.
    pub fn invalid_fields(fields: Vec<FieldError>) -> Self {
        let mut problem = Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ValidationFailed,
        );
        problem.fields = fields
            .into_iter()
            .map(|field| FieldProblem {
                field: field.field,
                code: field.code,
                params: field.params,
            })
            .collect();
        problem
    }

    pub fn invalid(field: FieldError) -> Self {
        let mut problem = Self::new(StatusCode::UNPROCESSABLE_ENTITY, field.code);
        problem.params.clone_from(&field.params);
        problem.fields.push(FieldProblem {
            field: field.field,
            code: field.code,
            params: field.params,
        });
        problem
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut response = (status, Json(self)).into_response();
        response.headers_mut().insert(CONTENT_TYPE, PROBLEM_JSON);
        response
    }
}

pub fn error_response(status: StatusCode, code: ErrorCode) -> Response {
    Problem::new(status, code).into_response()
}

/// Erreur interne : détail journalisé, jamais renvoyé au client.
pub fn internal_error(error: &dyn std::fmt::Display, context: &'static str) -> Response {
    tracing::error!(%error, "{context}");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::InternalError)
}

/// Erreur de handler : toute erreur métier convertible en réponse, ou une réponse déjà prête
/// (refus du garde de permission). Permet `?` sur les deux. En boîte : un `Result` de handler
/// reste de la taille d'un pointeur (`clippy::result_large_err`).
#[derive(Debug)]
pub struct ApiError(Box<Response>);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        *self.0
    }
}

impl From<Response> for ApiError {
    fn from(response: Response) -> Self {
        Self(Box::new(response))
    }
}

impl From<Problem> for ApiError {
    fn from(problem: Problem) -> Self {
        Self::from(problem.into_response())
    }
}
