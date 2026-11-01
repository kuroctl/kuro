//! Second facteur : vérification (code TOTP ou code de secours), codes de secours, et gestion
//! du TOTP depuis son compte (activation, remplacement, désactivation).
//!
//! Toute gestion depuis le compte exige le mot de passe, et le second facteur actuel si le TOTP
//! est actif : une session volée ne suffit pas à le remplacer. Les échecs comptent pour le
//! verrouillage, comme à la connexion. Un refus est un `422` sur le champ fautif, jamais un `401`,
//! que le panel prendrait pour un jeton d'accès expiré.

use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use super::login::{TotpSetup, check_password, fail_attempt};
use super::tokens;
use super::{IamError, IamState};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::{recovery_code, secret_box, totp};
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::Principal;
use crate::iam::infra::{Credentials, RecoveryCodeRepository, SessionRepository, UserRepository};

/// Second facteur accepté.
#[derive(Debug, Clone, Copy)]
pub(super) enum SecondFactor {
    /// Code TOTP, avec son pas de temps (à mémoriser contre le rejeu).
    Totp(i64),
    /// Code de secours consommé ; nombre de codes restants.
    RecoveryCode { remaining: i64 },
}

impl SecondFactor {
    /// Pas TOTP à mémoriser.
    pub(super) fn totp_step(self) -> Option<i64> {
        match self {
            Self::Totp(step) => Some(step),
            Self::RecoveryCode { .. } => None,
        }
    }

    /// Valeur `mfa` des événements de connexion.
    pub(super) fn kind(self) -> &'static str {
        match self {
            Self::Totp(_) => "totp",
            Self::RecoveryCode { .. } => "recovery_code",
        }
    }

    /// Événement à joindre quand un code de secours a été consommé.
    pub(super) fn audit(self, user_id: Uuid) -> Option<AuditEvent> {
        match self {
            Self::Totp(_) => None,
            Self::RecoveryCode { remaining } => Some(
                AuditEvent::success(AuditAction::RecoveryCodeUsed)
                    .target("admin_user", user_id)
                    .details(json!({ "remaining": remaining })),
            ),
        }
    }
}

/// Vérifie `code` pour un compte dont le TOTP est actif : un code de secours (reconnu à son
/// format) est consommé dans la transaction, un code TOTP doit être postérieur au dernier
/// accepté. `None` : code refusé.
pub(super) async fn check_second_factor(
    conn: &mut PgConnection,
    state: &IamState,
    credentials: &Credentials,
    code: &str,
) -> Result<Option<SecondFactor>, IamError> {
    if let Some(canonical) = recovery_code::normalize(code) {
        let hash = recovery_code::hash(state.keys.recovery_codes(), &canonical);
        let remaining = RecoveryCodeRepository::consume(conn, credentials.id, &hash).await?;
        return Ok(remaining.map(|remaining| SecondFactor::RecoveryCode { remaining }));
    }
    let Some(sealed) = credentials.totp_secret.as_deref() else {
        return Ok(None);
    };
    let secret = secret_box::open(state.keys.totp(), sealed)?;
    let last_step = credentials
        .totp_last_step
        .and_then(|step| u64::try_from(step).ok());
    let step = totp::verify(&secret, code, tokens::now(), last_step)?;
    Ok(step
        .and_then(|step| i64::try_from(step).ok())
        .map(SecondFactor::Totp))
}

/// Nouveaux codes de secours (les précédents sont effacés) ; retournés au format d'affichage,
/// seule fois où ils sont lisibles.
pub(super) async fn issue_recovery_codes(
    conn: &mut PgConnection,
    state: &IamState,
    user_id: Uuid,
) -> Result<Vec<String>, IamError> {
    let codes = (0..recovery_code::COUNT)
        .map(|_| recovery_code::generate())
        .collect::<Result<Vec<_>, _>>()?;
    let hashes: Vec<Vec<u8>> = codes
        .iter()
        .map(|code| recovery_code::hash(state.keys.recovery_codes(), code))
        .collect();
    RecoveryCodeRepository::replace(conn, user_id, &hashes).await?;
    Ok(codes
        .iter()
        .map(|code| recovery_code::display(code))
        .collect())
}

/// Commence la configuration du TOTP depuis son compte : premier TOTP, ou remplacement (nouveau
/// téléphone). Le secret attend sa confirmation ; l'actuel reste valide jusque-là.
pub async fn start_own_totp_setup(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    password: &str,
    code: Option<&str>,
) -> Result<TotpSetup, IamError> {
    let (mut tx, credentials) = reauthenticate(state, principal, context, password, code).await?;
    let secret = totp::generate_secret()?;
    let setup = TotpSetup {
        secret: totp::secret_base32(&secret),
        otpauth_uri: totp::otpauth_uri(&secret, &credentials.email)?,
    };
    UserRepository::set_pending_totp(
        &mut tx,
        principal.user_id,
        &secret_box::seal(state.keys.totp(), &secret)?,
    )
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::TotpSetupStarted)
            .target("admin_user", principal.user_id)
            .details(json!({ "replace": credentials.totp_secret.is_some() })),
    )
    .await?;
    Ok(setup)
}

/// Confirme le secret en attente par un premier code : il devient le TOTP du compte, avec de
/// nouveaux codes de secours. Un remplacement ferme les autres sessions.
pub async fn confirm_own_totp(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    code: &str,
) -> Result<Vec<String>, IamError> {
    let user_id = principal.user_id;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = own_credentials(&mut tx, user_id).await?;
    let sealed = credentials
        .totp_pending_secret
        .as_deref()
        .ok_or(IamError::TotpSetupNotStarted)?;
    let secret = secret_box::open(state.keys.totp(), sealed)?;
    let step =
        totp::verify(&secret, code, tokens::now(), None)?.and_then(|step| i64::try_from(step).ok());
    let Some(step) = step else {
        return Err(fail_attempt(tx, user_id, AuditAction::TotpFailed, invalid_code()).await);
    };

    let replaced = credentials.totp_secret.is_some();
    UserRepository::enable_totp(&mut tx, user_id, step).await?;
    UserRepository::record_second_factor(&mut tx, user_id, None).await?;
    let codes = issue_recovery_codes(&mut tx, state, user_id).await?;
    let event = if replaced {
        let revoked =
            SessionRepository::revoke_user(&mut tx, user_id, Some(principal.session_id)).await?;
        AuditEvent::success(AuditAction::TotpReplaced)
            .details(json!({ "revoked_sessions": revoked }))
    } else {
        AuditEvent::success(AuditAction::TotpEnabled)
    };
    tx.commit(event.target("admin_user", user_id)).await?;
    Ok(codes)
}

/// Nouveaux codes de secours ; les précédents, utilisés ou non, ne valent plus rien.
pub async fn regenerate_recovery_codes(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    password: &str,
    code: Option<&str>,
) -> Result<Vec<String>, IamError> {
    let (mut tx, credentials) = reauthenticate(state, principal, context, password, code).await?;
    if credentials.totp_secret.is_none() {
        return Err(IamError::Conflict(ErrorCode::TotpNotEnabled));
    }
    let codes = issue_recovery_codes(&mut tx, state, principal.user_id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::RecoveryCodesRegenerated)
            .target("admin_user", principal.user_id),
    )
    .await?;
    Ok(codes)
}

/// Retire son TOTP et ses codes de secours. Refusé quand la plateforme l'exige
/// (`auth.require_totp`) : le compte devrait aussitôt le reconfigurer.
pub async fn disable_own_totp(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    password: &str,
    code: Option<&str>,
) -> Result<(), IamError> {
    if state.settings.require_totp {
        return Err(IamError::Forbidden(ErrorCode::TotpRequiredByPolicy));
    }
    let (mut tx, credentials) = reauthenticate(state, principal, context, password, code).await?;
    if credentials.totp_secret.is_none() {
        return Err(IamError::Conflict(ErrorCode::TotpNotEnabled));
    }
    UserRepository::reset_totp(&mut tx, principal.user_id).await?;
    RecoveryCodeRepository::delete_all(&mut tx, principal.user_id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::TotpDisabled).target("admin_user", principal.user_id),
    )
    .await?;
    Ok(())
}

/// Mot de passe, puis second facteur si le TOTP est actif. Retourne la transaction ouverte (le
/// code de secours éventuel y est consommé et journalisé) et les identifiants lus.
async fn reauthenticate(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    password: &str,
    code: Option<&str>,
) -> Result<(AuditedTx, Credentials), IamError> {
    let user_id = principal.user_id;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = own_credentials(&mut tx, user_id).await?;
    if !check_password(password, &credentials.password_hash).await {
        let error = IamError::Invalid(FieldError::new("password", ErrorCode::InvalidCredentials));
        return Err(fail_attempt(tx, user_id, AuditAction::LoginFailed, error).await);
    }
    if credentials.totp_secret.is_none() {
        return Ok((tx, credentials));
    }

    let code = code.map(str::trim).filter(|code| !code.is_empty());
    let Some(code) = code else {
        return Err(IamError::Invalid(FieldError::new(
            "code",
            ErrorCode::TotpCodeRequired,
        )));
    };
    let Some(factor) = check_second_factor(&mut tx, state, &credentials, code).await? else {
        return Err(fail_attempt(tx, user_id, AuditAction::TotpFailed, invalid_code()).await);
    };
    UserRepository::record_second_factor(&mut tx, user_id, factor.totp_step()).await?;
    if let Some(event) = factor.audit(user_id) {
        tx.record(event).await?;
    }
    Ok((tx, credentials))
}

/// Identifiants du compte connecté. Verrouillé : refusé comme un mauvais mot de passe, pour que
/// la session ne contourne pas le verrouillage.
pub(super) async fn own_credentials(
    tx: &mut AuditedTx,
    user_id: Uuid,
) -> Result<Credentials, IamError> {
    let credentials = UserRepository::credentials_by_id(tx, user_id)
        .await?
        .ok_or(IamError::InvalidSession)?;
    if credentials.locked {
        return Err(IamError::Invalid(FieldError::new(
            "password",
            ErrorCode::InvalidCredentials,
        )));
    }
    Ok(credentials)
}

pub(super) fn invalid_code() -> IamError {
    IamError::Invalid(FieldError::new("code", ErrorCode::InvalidCode))
}
