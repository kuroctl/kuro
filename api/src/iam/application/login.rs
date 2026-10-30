//! Connexion : mot de passe, son remplacement s'il a été choisi par un autre admin, puis TOTP
//! (vérification ou première configuration).

use serde_json::json;
use uuid::Uuid;

use super::tokens::{self, Challenge, ChallengePurpose, SessionTokens};
use super::{IamError, IamState, LOCK_DURATION, MAX_FAILED_ATTEMPTS};
use crate::audit::domain::{Actor, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::{password, secret_box, totp};
use crate::http::error::{ErrorCode, FieldError};
use crate::iam::domain::Email;
use crate::iam::infra::{Credentials, UserRepository};

#[derive(Debug)]
pub enum LoginOutcome {
    Authenticated(SessionTokens),
    /// Mot de passe valide mais choisi par un autre admin : à remplacer avant tout le reste.
    PasswordChangeRequired(Challenge),
    /// Mot de passe valide, code TOTP attendu.
    TotpRequired(Challenge),
    /// Mot de passe valide, TOTP obligatoire mais pas encore configuré.
    TotpSetupRequired(Challenge),
}

/// Secret à enregistrer dans l'application d'authentification.
#[derive(Debug)]
pub struct TotpSetup {
    pub secret: String,
    pub otpauth_uri: String,
}

pub async fn login(
    state: &IamState,
    context: &AuditContext,
    email: &str,
    password: &str,
) -> Result<LoginOutcome, IamError> {
    // Le hachage Argon2 (volontairement lent) ne tient aucune connexion : la lecture est rendue
    // au pool avant, la transaction d'audit n'est ouverte qu'après.
    let mut conn = state.pool.acquire().await?;
    let credentials = match Email::parse(email) {
        Ok(email) => UserRepository::credentials_by_email(&mut conn, email.as_str()).await?,
        Err(_) => None,
    };
    drop(conn);

    let Some(credentials) = credentials else {
        verify_password(password, password::dummy_hash()).await;
        let tx = AuditedTx::begin(&state.pool, context).await?;
        tx.commit(
            AuditEvent::failure(AuditAction::LoginFailed)
                .details(json!({ "reason": "unknown_account" })),
        )
        .await?;
        return Err(IamError::InvalidCredentials);
    };
    if let Some(reason) = refusal(&credentials) {
        verify_password(password, password::dummy_hash()).await;
        let tx = AuditedTx::begin(&state.pool, context).await?;
        tx.commit(
            AuditEvent::failure(AuditAction::LoginFailed)
                .target("admin_user", credentials.id)
                .details(json!({ "reason": reason })),
        )
        .await?;
        return Err(IamError::InvalidCredentials);
    }
    if !verify_password(password, &credentials.password_hash).await {
        let tx = AuditedTx::begin(&state.pool, context).await?;
        return Err(fail_attempt(
            tx,
            credentials.id,
            AuditAction::LoginFailed,
            IamError::InvalidCredentials,
        )
        .await);
    }

    let tx = AuditedTx::begin(&state.pool, context).await?;
    // Mot de passe valide : aucune session tant qu'un mot de passe temporaire n'est pas remplacé.
    if credentials.password_change_required {
        tx.commit(
            AuditEvent::success(AuditAction::LoginPasswordChangeRequired)
                .target("admin_user", credentials.id),
        )
        .await?;
        return Ok(LoginOutcome::PasswordChangeRequired(tokens::challenge(
            state,
            credentials.id,
            ChallengePurpose::ChangePassword,
        )));
    }
    after_password(tx, state, &credentials, None).await
}

/// Remplace le mot de passe choisi par un autre admin, puis poursuit la connexion (TOTP ou
/// session). L'ancien mot de passe a été vérifié à l'étape précédente.
pub async fn change_required_password(
    state: &IamState,
    context: &AuditContext,
    challenge: &str,
    new_password: &str,
) -> Result<LoginOutcome, IamError> {
    let user_id = tokens::read_challenge(state, challenge, ChallengePurpose::ChangePassword)?;
    password::check_policy(new_password)
        .map_err(|_| IamError::Invalid(FieldError::new("new_password", ErrorCode::WeakPassword)))?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = usable_credentials(&mut tx, user_id).await?;
    // Déjà remplacé : le challenge ne sert pas deux fois.
    if !credentials.password_change_required {
        return Err(IamError::InvalidChallenge);
    }
    if verify_password(new_password, &credentials.password_hash).await {
        return Err(IamError::Invalid(FieldError::new(
            "new_password",
            ErrorCode::PasswordUnchanged,
        )));
    }

    let password_hash = hash_password(new_password).await?;
    UserRepository::set_password(&mut tx, user_id, &password_hash, false).await?;
    let changed = AuditEvent::success(AuditAction::OwnPasswordChanged)
        .target("admin_user", user_id)
        .details(json!({ "forced": true }));
    after_password(tx, state, &credentials, Some(changed)).await
}

/// Étape suivant un mot de passe accepté : TOTP s'il est configuré ou exigé, sinon session.
/// `done` : événement déjà acquis (mot de passe remplacé), validé avec cette étape.
async fn after_password(
    mut tx: AuditedTx,
    state: &IamState,
    credentials: &Credentials,
    done: Option<AuditEvent>,
) -> Result<LoginOutcome, IamError> {
    let user_id = credentials.id;
    if credentials.totp_secret.is_some() || state.settings.require_totp {
        if let Some(event) = done {
            tx.set_actor(Actor::admin(user_id));
            tx.commit(event).await?;
        }
        return Ok(if credentials.totp_secret.is_some() {
            LoginOutcome::TotpRequired(tokens::challenge(
                state,
                user_id,
                ChallengePurpose::VerifyTotp,
            ))
        } else {
            LoginOutcome::TotpSetupRequired(tokens::challenge(
                state,
                user_id,
                ChallengePurpose::SetupTotp,
            ))
        });
    }

    UserRepository::record_success(&mut tx, user_id, None).await?;
    let session = tokens::open_session(&mut tx, state, user_id).await?;
    tx.set_actor(Actor::admin(user_id));
    if let Some(event) = done {
        tx.record(event).await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::LoginSucceeded)
            .target("admin_user", user_id)
            .details(json!({ "mfa": false })),
    )
    .await?;
    Ok(LoginOutcome::Authenticated(session))
}

pub async fn verify_totp(
    state: &IamState,
    context: &AuditContext,
    challenge: &str,
    code: &str,
) -> Result<SessionTokens, IamError> {
    let user_id = tokens::read_challenge(state, challenge, ChallengePurpose::VerifyTotp)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = usable_credentials(&mut tx, user_id).await?;
    let sealed = credentials
        .totp_secret
        .as_deref()
        .ok_or(IamError::InvalidChallenge)?;
    let secret = secret_box::open(state.keys.totp(), sealed)?;

    let last_step = credentials
        .totp_last_step
        .and_then(|step| u64::try_from(step).ok());
    let Some(step) = totp::verify(&secret, code, tokens::now(), last_step)? else {
        return Err(
            fail_attempt(tx, user_id, AuditAction::TotpFailed, IamError::InvalidCode).await,
        );
    };

    let step = i64::try_from(step).map_err(|_| IamError::InvalidCode)?;
    UserRepository::record_success(&mut tx, user_id, Some(step)).await?;
    let session = tokens::open_session(&mut tx, state, user_id).await?;
    tx.set_actor(Actor::admin(user_id));
    tx.commit(
        AuditEvent::success(AuditAction::LoginSucceeded)
            .target("admin_user", user_id)
            .details(json!({ "mfa": "totp" })),
    )
    .await?;
    Ok(session)
}

/// Première étape de configuration : secret généré, stocké chiffré en attente de confirmation.
pub async fn start_totp_setup(
    state: &IamState,
    context: &AuditContext,
    challenge: &str,
) -> Result<TotpSetup, IamError> {
    let user_id = tokens::read_challenge(state, challenge, ChallengePurpose::SetupTotp)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = usable_credentials(&mut tx, user_id).await?;
    // Sans ce contrôle, un mot de passe seul suffirait à remplacer un TOTP existant.
    if credentials.totp_secret.is_some() {
        return Err(IamError::TotpAlreadyEnabled);
    }

    let secret = totp::generate_secret()?;
    let setup = TotpSetup {
        secret: totp::secret_base32(&secret),
        otpauth_uri: totp::otpauth_uri(&secret, &credentials.email)?,
    };
    UserRepository::set_pending_totp(
        &mut tx,
        user_id,
        &secret_box::seal(state.keys.totp(), &secret)?,
    )
    .await?;
    tx.set_actor(Actor::admin(user_id));
    tx.commit(AuditEvent::success(AuditAction::TotpSetupStarted).target("admin_user", user_id))
        .await?;
    Ok(setup)
}

/// Seconde étape : un premier code valide active le TOTP et ouvre la session.
pub async fn confirm_totp(
    state: &IamState,
    context: &AuditContext,
    challenge: &str,
    code: &str,
) -> Result<SessionTokens, IamError> {
    let user_id = tokens::read_challenge(state, challenge, ChallengePurpose::SetupTotp)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = usable_credentials(&mut tx, user_id).await?;
    if credentials.totp_secret.is_some() {
        return Err(IamError::TotpAlreadyEnabled);
    }
    let sealed = credentials
        .totp_pending_secret
        .as_deref()
        .ok_or(IamError::TotpSetupNotStarted)?;
    let secret = secret_box::open(state.keys.totp(), sealed)?;

    let Some(step) = totp::verify(&secret, code, tokens::now(), None)? else {
        return Err(
            fail_attempt(tx, user_id, AuditAction::TotpFailed, IamError::InvalidCode).await,
        );
    };

    let step = i64::try_from(step).map_err(|_| IamError::InvalidCode)?;
    UserRepository::enable_totp(&mut tx, user_id, step).await?;
    UserRepository::record_success(&mut tx, user_id, Some(step)).await?;
    let session = tokens::open_session(&mut tx, state, user_id).await?;
    tx.set_actor(Actor::admin(user_id));
    tx.record(AuditEvent::success(AuditAction::TotpEnabled).target("admin_user", user_id))
        .await?;
    tx.commit(
        AuditEvent::success(AuditAction::LoginSucceeded)
            .target("admin_user", user_id)
            .details(json!({ "mfa": "totp" })),
    )
    .await?;
    Ok(session)
}

/// Compte pouvant poursuivre une connexion (ni désactivé, ni verrouillé).
async fn usable_credentials(tx: &mut AuditedTx, user_id: Uuid) -> Result<Credentials, IamError> {
    let credentials = UserRepository::credentials_by_id(tx, user_id)
        .await?
        .ok_or(IamError::InvalidChallenge)?;
    if refusal(&credentials).is_some() {
        return Err(IamError::InvalidCredentials);
    }
    Ok(credentials)
}

fn refusal(credentials: &Credentials) -> Option<&'static str> {
    if credentials.disabled {
        Some("disabled")
    } else if credentials.locked {
        Some("locked")
    } else {
        None
    }
}

/// Échec compté (verrouillage au-delà du seuil) et journalisé ; retourne l'erreur à renvoyer.
async fn fail_attempt(
    mut tx: AuditedTx,
    user_id: Uuid,
    action: AuditAction,
    error: IamError,
) -> IamError {
    let result = async {
        let locked = UserRepository::record_failure(
            &mut tx,
            user_id,
            MAX_FAILED_ATTEMPTS,
            LOCK_DURATION.as_secs_f64(),
        )
        .await?;
        let action = if locked {
            AuditAction::AccountLocked
        } else {
            action
        };
        tx.commit(AuditEvent::failure(action).target("admin_user", user_id))
            .await
    }
    .await;
    match result {
        Ok(()) => error,
        Err(db) => db.into(),
    }
}

/// Vérification Argon2 hors du runtime async (calcul volontairement coûteux).
async fn verify_password(password: &str, hash: &str) -> bool {
    let (password, hash) = (password.to_owned(), hash.to_owned());
    tokio::task::spawn_blocking(move || password::verify(&password, &hash))
        .await
        .unwrap_or(false)
}

/// Hachage Argon2 hors du runtime async.
pub(super) async fn hash_password(password: &str) -> Result<String, IamError> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || password::hash(&password))
        .await
        .map_err(|error| IamError::Internal(error.to_string()))?
        .map_err(Into::into)
}

pub(super) async fn check_password(password: &str, hash: &str) -> bool {
    verify_password(password, hash).await
}
