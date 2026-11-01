//! Propriétaire d'un serveur (ADR-0051) : posé à la création ou transféré par un admin, jamais
//! vérifié contre une liste de comptes (l'extension qui l'a posé en répond).

use serde_json::json;
use uuid::Uuid;

use super::{ServerView, ServersError, ServersState, get};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::servers::domain::{ServerEvent, is_owner_ref};
use crate::servers::infra::ServerRepository;

/// Propriétaire reçu d'un client : `None` reste `None`, sinon le format est vérifié.
pub(super) fn checked(owner_ref: Option<String>) -> Result<Option<String>, ServersError> {
    match owner_ref {
        Some(value) if !is_owner_ref(&value) => Err(ServersError::Invalid(FieldError::new(
            "owner_ref",
            ErrorCode::InvalidOwnerRef,
        ))),
        other => Ok(other),
    }
}

/// Change le propriétaire (transfert) ; `None` rend le serveur interne. Refusé s'il ne change
/// pas (`empty_patch`). Audité avec l'ancien et le nouveau propriétaire, identifiants opaques.
pub async fn set_owner(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    owner_ref: Option<String>,
) -> Result<ServerView, ServersError> {
    let owner_ref = checked(owner_ref)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let previous = ServerRepository::owner_for_update(&mut tx, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    if previous == owner_ref {
        return Err(ServersError::Unprocessable(ErrorCode::EmptyPatch));
    }
    ServerRepository::set_owner(&mut tx, id, owner_ref.as_deref()).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerOwnerChanged)
            .target("server", id)
            .details(json!({ "from": previous, "to": owner_ref })),
    )
    .await?;
    state.events.emit(ServerEvent::OwnerChanged {
        server_id: id,
        from: previous,
        to: owner_ref,
    });
    get(state, id).await
}
