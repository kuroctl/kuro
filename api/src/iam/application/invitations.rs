//! Invitations des admins (ADR-0046) : un lien à usage unique, envoyé par e-mail, qui laisse
//! l'invité choisir son mot de passe ; puis la connexion habituelle (TOTP compris). Le jeton
//! n'est ni journalisé ni stocké en clair.

use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::login::{self, LoginOutcome};
use super::{IamError, IamState};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::{opaque, password};
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::Authority;
use crate::iam::infra::{InvitationRepository, SessionRepository, UserRepository};
use crate::mail::{MailError, MailState, Message};

const TOKEN_PREFIX: &str = "kuro_inv_";
/// Durée de validité d'une invitation.
pub const TTL: Duration = Duration::hours(48);

/// Invite (ou réinvite) un compte : nouveau jeton, l'ancien révoqué, e-mail envoyé avant de
/// valider. Exige un SMTP réglé.
pub async fn invite(
    state: &IamState,
    mail: &MailState,
    authority: &Authority,
    context: &AuditContext,
    user_id: Uuid,
) -> Result<(), IamError> {
    if authority.is_user(user_id) {
        return Err(IamError::Forbidden(ErrorCode::UseOwnPasswordChange));
    }
    if !mail
        .configured()
        .await
        .map_err(|error| IamError::Internal(error.to_string()))?
    {
        return Err(IamError::Unprocessable(ErrorCode::MailNotConfigured));
    }
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let user = UserRepository::get(&mut tx, user_id)
        .await?
        .ok_or(IamError::NotFound)?;
    if user.disabled {
        return Err(IamError::Conflict(ErrorCode::UserDisabled));
    }
    super::users::ensure_manageable(&mut tx, authority, user_id).await?;
    let token = opaque::generate(TOKEN_PREFIX).map_err(|e| IamError::Internal(e.to_string()))?;
    let expires_at = OffsetDateTime::now_utc() + TTL;
    InvitationRepository::revoke_open(&mut tx, user_id).await?;
    InvitationRepository::insert(
        &mut tx,
        Uuid::now_v7(),
        user_id,
        &opaque::hash(&token),
        context.actor.id,
        expires_at,
    )
    .await?;

    let brand = mail
        .brand()
        .await
        .map_err(|error| IamError::Internal(error.to_string()))?;
    let link = match &mail.panel_url {
        Some(panel) => format!("{}/invitation/{token}", panel.trim_end_matches('/')),
        None => format!("/invitation/{token}"),
    };
    let email = crate::mail::Email {
        brand: brand.clone(),
        title: "Invitation".to_owned(),
        paragraphs: vec![
            format!("{},", user.display_name),
            format!(
                "Vous êtes invité·e à rejoindre {brand}. Choisissez votre mot de passe en ouvrant ce lien, valable 48 h et une seule fois."
            ),
            format!(
                "You are invited to join {brand}. Choose your password by opening this link, valid 48 hours and once."
            ),
        ],
        action: Some((
            "Choisir mon mot de passe · Choose my password".to_owned(),
            link,
        )),
        note: None,
    };
    let message = Message::from_template(vec![user.email.clone()], &email);
    // L'e-mail part avant la validation : une invitation que personne ne reçoit n'existe pas.
    match mail.send(message).await {
        Ok(()) => {}
        Err(MailError::NotConfigured) => {
            return Err(IamError::Unprocessable(ErrorCode::MailNotConfigured));
        }
        Err(MailError::Send(_)) => return Err(IamError::Unprocessable(ErrorCode::MailSendFailed)),
        Err(error) => return Err(IamError::Internal(error.to_string())),
    }
    tx.commit(
        AuditEvent::success(AuditAction::UserInvited)
            .target("admin_user", user_id)
            .details(json!({ "expires_at": expires_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default() })),
    )
    .await?;
    Ok(())
}

/// Consomme une invitation : pose le mot de passe choisi, ferme les sessions éventuelles, puis
/// poursuit comme une connexion (session ou TOTP à configurer).
pub async fn accept(
    state: &IamState,
    context: &AuditContext,
    token: &str,
    new_password: &str,
) -> Result<LoginOutcome, IamError> {
    password::check_policy(new_password)
        .map_err(|_| IamError::Invalid(FieldError::new("password", ErrorCode::WeakPassword)))?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let invitation = InvitationRepository::open_by_hash(&mut tx, &opaque::hash(token))
        .await?
        .ok_or(IamError::Forbidden(ErrorCode::InvalidToken))?;
    let user = UserRepository::get(&mut tx, invitation.user_id)
        .await?
        .ok_or(IamError::Forbidden(ErrorCode::InvalidToken))?;
    if user.disabled {
        return Err(IamError::Forbidden(ErrorCode::InvalidToken));
    }
    let password_hash = login::hash_password(new_password).await?;
    UserRepository::set_password(&mut tx, user.id, &password_hash, false).await?;
    SessionRepository::revoke_user(&mut tx, user.id, None).await?;
    InvitationRepository::mark_used(&mut tx, invitation.id).await?;
    tx.commit(AuditEvent::success(AuditAction::InvitationAccepted).target("admin_user", user.id))
        .await?;
    login::login(state, context, &user.email, new_password).await
}

/// Comptes dont une invitation est en attente.
pub async fn pending(state: &IamState) -> Result<Vec<Uuid>, IamError> {
    let mut conn = state.pool.acquire().await?;
    Ok(InvitationRepository::pending_user_ids(&mut conn).await?)
}
