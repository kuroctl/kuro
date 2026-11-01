//! Extracteur de session admin et garde de permission.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::audit::domain::{Actor, AuditAction, AuditContext, AuditEvent};
use crate::http::context::request_context;
use crate::http::error::{ApiError, ErrorCode, Problem, error_response};
use crate::iam::application::{IamState, authenticate, elevation};
use crate::iam::domain::{Authority, Permission, Principal};

/// Admin authentifié par `Authorization: Bearer <jeton d'accès>`.
#[derive(Debug, Clone)]
pub struct AdminSession {
    pub principal: Principal,
    /// Contexte d'audit de la requête, acteur identifié.
    pub context: AuditContext,
}

impl AdminSession {
    /// Garde de permission. Un refus est journalisé (`access.denied`) avant d'être renvoyé.
    pub async fn require(
        &self,
        state: &IamState,
        permission: Permission,
    ) -> Result<Authority, ApiError> {
        if self.principal.permissions.contains(permission) {
            return Ok(Authority::Admin(self.principal.clone()));
        }
        let event = AuditEvent::denied(AuditAction::AccessDenied)
            .details(json!({ "permission": permission.key() }));
        if let Err(error) =
            crate::audit::application::record(&state.pool, &self.context, event).await
        {
            tracing::error!(%error, "failed to record access denial");
        }
        Err(Problem::new(StatusCode::FORBIDDEN, ErrorCode::Forbidden)
            .param("permission", permission.key())
            .into())
    }

    /// Garde d'une action protégée (ADR-0050) : la permission d'abord (un refus reste un `403
    /// forbidden`), puis une élévation récente de la session. Sans elle : `403
    /// elevation_required`, `params.method` (`totp` ou `password`) dit au panel quoi demander,
    /// et `WWW-Authenticate` porte l'erreur de la RFC 9470. Pas de `401` : le panel le prendrait
    /// pour un jeton expiré. Le refus est journalisé (`access.denied`, raison
    /// `elevation_required`).
    pub async fn require_elevated(
        &self,
        state: &IamState,
        permission: Permission,
    ) -> Result<Authority, ApiError> {
        // Un agent IA ne peut pas s'élever : une action protégée ne doit jamais lui être ouverte.
        debug_assert!(!permission.agent_grantable(), "{permission:?}");
        let authority = self.require(state, permission).await?;
        let Some(method) = elevation::required(state, &self.principal).await? else {
            return Ok(authority);
        };
        let event = AuditEvent::denied(AuditAction::AccessDenied).details(json!({
            "permission": permission.key(),
            "reason": "elevation_required",
        }));
        if let Err(error) =
            crate::audit::application::record(&state.pool, &self.context, event).await
        {
            tracing::error!(%error, "failed to record access denial");
        }
        let mut response = Problem::new(StatusCode::FORBIDDEN, ErrorCode::ElevationRequired)
            .param("method", method.as_str())
            .into_response();
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Bearer error="insufficient_user_authentication""#),
        );
        Err(ApiError::from(response))
    }
}

impl<S> FromRequestParts<S> for AdminSession
where
    IamState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let unauthorized = || error_response(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized);
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(unauthorized)?;

        let iam = IamState::from_ref(state);
        let principal = authenticate(&iam, token)
            .await
            .map_err(IntoResponse::into_response)?;
        let context = request_context(parts).with_actor(Actor::admin(principal.user_id));
        crate::http::trace::record_actor(&context.actor);
        Ok(Self { principal, context })
    }
}
