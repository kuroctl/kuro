//! Jetons d'accès (validation), rafraîchissement avec rotation, déconnexion.

use std::time::Duration;

use serde_json::json;

use super::tokens::{self, ADMIN_AUDIENCE, AccessToken, SessionTokens};
use super::{IamError, IamState};
use crate::audit::domain::{Actor, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::{jwt, opaque, secret_box};
use crate::iam::domain::{PermissionSet, Principal};
use crate::iam::infra::{SessionRepository, UserRepository};

/// Valide un jeton d'accès et recharge les droits : une désactivation, une déconnexion ou un
/// retrait de rôle prend effet immédiatement, sans attendre l'expiration du jeton.
pub async fn authenticate(state: &IamState, access_token: &str) -> Result<Principal, IamError> {
    let claims = jwt::decode(
        state.keys.jwt(),
        access_token,
        ADMIN_AUDIENCE,
        tokens::now(),
    )
    .map_err(|_| IamError::InvalidSession)?;
    let session_id = claims.sid.ok_or(IamError::InvalidSession)?;

    let mut conn = state.pool.acquire().await?;
    if !UserRepository::has_active_session(&mut conn, claims.sub, session_id).await? {
        return Err(IamError::InvalidSession);
    }
    let keys = UserRepository::permission_keys(&mut conn, claims.sub).await?;
    Ok(Principal {
        user_id: claims.sub,
        session_id,
        permissions: PermissionSet::from_keys(keys),
    })
}

/// Délai pendant lequel un jeton tout juste remplacé reste accepté : deux onglets qui
/// rafraîchissent en même temps envoient le même cookie, sans qu'il y ait vol.
pub const ROTATION_GRACE: Duration = Duration::from_secs(10);

/// Issue d'un rafraîchissement.
#[derive(Debug)]
pub enum Refreshed {
    /// Cas normal : jeton consommé, nouveau jeton de rafraîchissement émis.
    Rotated(SessionTokens),
    /// Jeton remplacé à l'instant par une autre requête : on renvoie le même successeur, car la
    /// réponse de cette autre requête a pu se perdre (page rechargée pendant le rafraîchissement)
    /// et le navigateur garderait sinon un cookie consommé, pris ensuite pour un vol.
    Grace(SessionTokens),
    /// Même cas, successeur illisible (clé changée) : jeton d'accès seul.
    GraceAccessOnly(AccessToken),
}

/// Rotation : l'ancien jeton est consommé. Présenter un jeton déjà consommé, hors de la
/// fenêtre de grâce, signe un vol : toute la session est révoquée.
pub async fn refresh(
    state: &IamState,
    context: &AuditContext,
    refresh_token: &str,
) -> Result<Refreshed, IamError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let Some(session) = SessionRepository::find_for_update(
        &mut tx,
        &opaque::hash(refresh_token),
        ROTATION_GRACE,
        state.settings.session_max_age,
    )
    .await?
    else {
        return Err(IamError::InvalidSession);
    };
    tx.set_actor(Actor::admin(session.user_id));

    if session.revoked || session.expired || session.user_disabled {
        return Err(IamError::InvalidSession);
    }
    // Rafraîchie sans fin, une session durerait toujours : au-delà de la durée maximale, toute
    // la famille s'éteint et l'admin se reconnecte.
    if session.too_old {
        SessionRepository::revoke_family(&mut tx, session.family_id).await?;
        tx.commit(
            AuditEvent::failure(AuditAction::SessionRefreshed)
                .target("admin_session", session.family_id)
                .details(json!({ "reason": "max_age" })),
        )
        .await?;
        return Err(IamError::InvalidSession);
    }
    if session.rotated_recently {
        let token = tokens::access(state, session.user_id, session.family_id);
        let successor = session
            .successor_secret
            .as_deref()
            .and_then(|sealed| secret_box::open(state.keys.session_successors(), sealed).ok())
            .and_then(|plain| String::from_utf8(plain).ok());
        tx.commit(
            AuditEvent::success(AuditAction::SessionRefreshed)
                .target("admin_session", session.family_id)
                .details(json!({ "grace": true, "successor": successor.is_some() })),
        )
        .await?;
        return Ok(match successor {
            Some(refresh_token) => Refreshed::Grace(SessionTokens {
                access_token: token.access_token,
                expires_in: token.expires_in,
                refresh_token,
            }),
            None => Refreshed::GraceAccessOnly(token),
        });
    }
    if session.rotated {
        SessionRepository::revoke_family(&mut tx, session.family_id).await?;
        tx.commit(
            AuditEvent::failure(AuditAction::SessionReuseDetected)
                .target("admin_session", session.family_id),
        )
        .await?;
        return Err(IamError::InvalidSession);
    }

    SessionRepository::forget_successors(&mut tx, ROTATION_GRACE).await?;
    let tokens = tokens::issue(&mut tx, state, session.user_id, session.family_id).await?;
    let sealed = secret_box::seal(
        state.keys.session_successors(),
        tokens.refresh_token.as_bytes(),
    )
    .map_err(|error| IamError::Internal(error.to_string()))?;
    SessionRepository::mark_rotated(&mut tx, session.id, &sealed).await?;
    tx.commit(
        AuditEvent::success(AuditAction::SessionRefreshed)
            .target("admin_session", session.family_id),
    )
    .await?;
    Ok(Refreshed::Rotated(tokens))
}

pub async fn logout(
    state: &IamState,
    context: &AuditContext,
    principal: &Principal,
) -> Result<(), IamError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    SessionRepository::revoke_family(&mut tx, principal.session_id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::Logout).target("admin_session", principal.session_id),
    )
    .await?;
    Ok(())
}
