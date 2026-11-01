//! Élévation de session (ADR-0050) : les actions protégées (suppression d'un serveur, d'un nœud,
//! secrets des réglages…) exigent, en plus de leur permission, une preuve récente de l'identité
//! de l'admin. Code TOTP (ou code de secours) si le TOTP est configuré, mot de passe sinon ;
//! l'élévation vaut [`TTL`] pour la session qui l'a obtenue.

use std::time::Duration;

use serde_json::json;

use super::login::{check_password, fail_attempt};
use super::mfa::{check_second_factor, invalid_code, own_credentials};
use super::{IamError, IamState};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::domain::Principal;
use crate::iam::infra::{ElevationRepository, UserRepository};

/// Durée d'une élévation.
pub const TTL: Duration = Duration::from_secs(5 * 60);

/// Preuve demandée pour élever la session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationMethod {
    Totp,
    Password,
}

impl ElevationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Totp => "totp",
            Self::Password => "password",
        }
    }
}

/// `None` : session élevée ; sinon, la preuve à demander.
pub async fn required(
    state: &IamState,
    principal: &Principal,
) -> Result<Option<ElevationMethod>, IamError> {
    let mut conn = state.pool.acquire().await?;
    let (elevated, totp) =
        ElevationRepository::status(&mut conn, principal.session_id, principal.user_id).await?;
    Ok((!elevated).then_some(if totp {
        ElevationMethod::Totp
    } else {
        ElevationMethod::Password
    }))
}

/// Élève la session : code (TOTP ou code de secours) si le TOTP est actif, mot de passe sinon.
/// Les échecs comptent pour le verrouillage ; un refus est un `422` sur le champ fautif.
pub async fn elevate(
    state: &IamState,
    principal: &Principal,
    context: &AuditContext,
    password: Option<&str>,
    code: Option<&str>,
) -> Result<Duration, IamError> {
    let user_id = principal.user_id;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let credentials = own_credentials(&mut tx, user_id).await?;

    let method = if credentials.totp_secret.is_some() {
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
        factor.kind()
    } else {
        let password = password.unwrap_or_default();
        if !check_password(password, &credentials.password_hash).await {
            let error =
                IamError::Invalid(FieldError::new("password", ErrorCode::InvalidCredentials));
            return Err(fail_attempt(tx, user_id, AuditAction::LoginFailed, error).await);
        }
        UserRepository::record_second_factor(&mut tx, user_id, None).await?;
        ElevationMethod::Password.as_str()
    };

    ElevationRepository::grant(&mut tx, principal.session_id, user_id, TTL.as_secs_f64()).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ElevationGranted)
            .target("admin_user", user_id)
            .details(json!({ "method": method, "expires_in": TTL.as_secs() })),
    )
    .await?;
    Ok(TTL)
}
