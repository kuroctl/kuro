//! Mot de passe oublié (ADR-0049) : un lien à usage unique, envoyé par e-mail, laisse choisir un
//! nouveau mot de passe ; puis la connexion habituelle, **TOTP compris** : le lien remplace le
//! mot de passe, jamais le second facteur.
//!
//! La demande répond toujours de la même façon, compte existant ou non, et l'e-mail part en tâche
//! de fond : ni la réponse ni sa durée ne disent qui a un compte. Le jeton n'est ni journalisé ni
//! stocké en clair.

use serde_json::json;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::login::{self, LoginOutcome};
use super::{IamError, IamState};
use crate::audit::domain::{Actor, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::{opaque, password};
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::Email;
use crate::iam::infra::{PasswordResetRepository, SessionRepository, UserRepository};
use crate::mail::{MailState, Message};

const TOKEN_PREFIX: &str = "kuro_pwd_";
/// Durée de validité d'un lien.
pub const TTL: Duration = Duration::minutes(30);
/// Délai minimal entre deux demandes pour un même compte : au-delà, aucun e-mail ne part.
const COOLDOWN_SECS: f64 = 120.0;

/// Demande un lien de réinitialisation pour cette adresse. Toujours `Ok` pour une adresse bien
/// formée : compte inconnu, désactivé, SMTP absent ou demande trop rapprochée ne se distinguent
/// que dans le journal d'audit.
pub async fn request(
    state: &IamState,
    mail: &MailState,
    context: &AuditContext,
    email: &str,
) -> Result<(), IamError> {
    let email = Email::parse(email)
        .map_err(|_| IamError::Invalid(FieldError::new("email", ErrorCode::InvalidEmail)))?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let user = match UserRepository::id_by_email(&mut tx, email.as_str()).await? {
        Some(id) => UserRepository::get(&mut tx, id).await?,
        None => None,
    };
    let Some(user) = user else {
        return refuse(tx, None, "unknown_account").await;
    };
    if user.disabled {
        return refuse(tx, Some(user.id), "disabled").await;
    }
    let configured = mail
        .configured()
        .await
        .map_err(|error| IamError::Internal(error.to_string()))?;
    if !configured {
        return refuse(tx, Some(user.id), "mail_not_configured").await;
    }
    if PasswordResetRepository::requested_within(&mut tx, user.id, COOLDOWN_SECS).await? {
        return refuse(tx, Some(user.id), "too_soon").await;
    }

    let token = opaque::generate(TOKEN_PREFIX).map_err(|e| IamError::Internal(e.to_string()))?;
    let expires_at = OffsetDateTime::now_utc() + TTL;
    PasswordResetRepository::revoke_open(&mut tx, user.id).await?;
    PasswordResetRepository::insert(
        &mut tx,
        Uuid::now_v7(),
        user.id,
        &opaque::hash(&token),
        expires_at,
    )
    .await?;
    let brand = mail
        .brand()
        .await
        .map_err(|error| IamError::Internal(error.to_string()))?;
    tx.commit(
        AuditEvent::success(AuditAction::PasswordResetRequested)
            .target("admin_user", user.id)
            .details(json!({ "expires_at": expires_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default() })),
    )
    .await?;

    let message = Message::from_template(
        vec![user.email.clone()],
        &reset_email(mail, &brand, &user.display_name, &token),
    );
    // Hors de la requête : la latence SMTP trahirait l'existence du compte. Un échec ne se voit
    // que dans les journaux ; l'admin redemande un lien.
    let mail = mail.clone();
    let user_id = user.id;
    tokio::spawn(async move {
        if let Err(error) = mail.send(message).await {
            tracing::warn!(%user_id, %error, "password reset e-mail not sent");
        }
    });
    Ok(())
}

/// Consomme le lien : pose le nouveau mot de passe, ferme les sessions, puis poursuit comme une
/// connexion (TOTP attendu s'il est configuré). Le verrouillage éventuel n'est **pas** levé :
/// sinon, la boîte e-mail seule permettrait d'essayer sans fin des codes TOTP.
pub async fn complete(
    state: &IamState,
    context: &AuditContext,
    token: &str,
    new_password: &str,
) -> Result<LoginOutcome, IamError> {
    password::check_policy(new_password)
        .map_err(|_| IamError::Invalid(FieldError::new("password", ErrorCode::WeakPassword)))?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let reset = PasswordResetRepository::open_by_hash(&mut tx, &opaque::hash(token))
        .await?
        .ok_or(IamError::Forbidden(ErrorCode::InvalidToken))?;
    let user = UserRepository::get(&mut tx, reset.user_id)
        .await?
        .ok_or(IamError::Forbidden(ErrorCode::InvalidToken))?;
    if user.disabled {
        return Err(IamError::Forbidden(ErrorCode::InvalidToken));
    }
    let password_hash = login::hash_password(new_password).await?;
    UserRepository::replace_password(&mut tx, user.id, &password_hash).await?;
    let revoked = SessionRepository::revoke_user(&mut tx, user.id, None).await?;
    PasswordResetRepository::mark_used(&mut tx, reset.id).await?;
    tx.set_actor(Actor::admin(user.id));
    tx.commit(
        AuditEvent::success(AuditAction::PasswordResetCompleted)
            .target("admin_user", user.id)
            .details(json!({ "revoked_sessions": revoked })),
    )
    .await?;
    login::login(state, context, &user.email, new_password).await
}

/// Demande sans suite, journalisée avec sa raison ; la réponse reste celle d'un succès.
async fn refuse(tx: AuditedTx, user_id: Option<Uuid>, reason: &str) -> Result<(), IamError> {
    let mut event = AuditEvent::failure(AuditAction::PasswordResetRequested)
        .details(json!({ "reason": reason }));
    if let Some(user_id) = user_id {
        event = event.target("admin_user", user_id);
    }
    tx.commit(event).await?;
    Ok(())
}

fn reset_email(mail: &MailState, brand: &str, name: &str, token: &str) -> crate::mail::Email {
    let link = match &mail.panel_url {
        Some(panel) => format!("{}/password/reset/{token}", panel.trim_end_matches('/')),
        None => format!("/password/reset/{token}"),
    };
    crate::mail::Email {
        brand: brand.to_owned(),
        title: "Mot de passe · Password".to_owned(),
        paragraphs: vec![
            format!("{name},"),
            "Une réinitialisation de votre mot de passe a été demandée. Choisissez-en un nouveau en ouvrant ce lien, valable 30 minutes et une seule fois.".to_owned(),
            "A password reset was requested for your account. Choose a new password by opening this link, valid 30 minutes and once.".to_owned(),
        ],
        action: Some((
            "Choisir un mot de passe · Choose a password".to_owned(),
            link,
        )),
        note: Some(
            "Pas à l’origine de cette demande ? Ignorez ce message : votre mot de passe ne change pas. · Did not ask for this? Ignore this message: your password stays the same.".to_owned(),
        ),
    }
}
