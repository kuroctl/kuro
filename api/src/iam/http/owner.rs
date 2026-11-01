//! Session de l'espace client (ADR-0051) : le propriétaire d'un serveur, identifié par le
//! fournisseur d'identité d'une extension. Kuro ne connaît de lui qu'un identifiant opaque.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::response::Response;

use crate::audit::domain::AuditContext;
use crate::extension::ClientIdentity;
use crate::http::context::request_context;
use crate::http::error::{ErrorCode, error_response};
use crate::servers::domain::OwnerRef;

#[derive(Debug, Clone)]
pub struct OwnerSession {
    pub owner: OwnerRef,
    /// Contexte d'audit de la requête : acteur `owner`, identifiant dans `details.owner_ref`.
    pub context: AuditContext,
}

impl OwnerSession {
    /// Le serveur de propriétaire `owner_ref` appartient-il à cette session ?
    pub fn owns(&self, owner_ref: Option<&str>) -> bool {
        owner_ref == Some(self.owner.as_str())
    }
}

/// Jeton `Bearer` refusé ou absent : `401 unauthorized`, comme pour l'espace admin. Un jeton
/// refusé n'est pas audité (le fournisseur en a la trace) : seulement journalisé.
impl<S> FromRequestParts<S> for OwnerSession
where
    ClientIdentity: FromRef<S>,
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
            .filter(|token| !token.is_empty())
            .ok_or_else(unauthorized)?;

        let identity = ClientIdentity::from_ref(state);
        // Routes montées seulement avec un fournisseur : ce cas ne se présente pas.
        let Some(provider) = identity.provider() else {
            return Err(error_response(StatusCode::NOT_FOUND, ErrorCode::NotFound));
        };
        match provider.authenticate(token).await {
            Ok(Some(owner)) => {
                let context = request_context(parts).as_owner(owner.as_str());
                crate::http::trace::record_actor(&context.actor);
                Ok(Self { owner, context })
            }
            Ok(None) => {
                tracing::info!("client token refused by the identity provider");
                Err(unauthorized())
            }
            Err(error) => {
                tracing::error!(%error, "identity provider failed");
                Err(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorCode::InternalError,
                ))
            }
        }
    }
}
