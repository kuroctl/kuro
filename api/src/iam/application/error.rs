use crate::auth::password::PasswordError;
use crate::auth::secret_box::SecretBoxError;
use crate::auth::totp::TotpError;
use crate::error_code::{ErrorCode, FieldError};

#[derive(Debug, thiserror::Error)]
pub enum IamError {
    /// Identifiant, mot de passe, compte désactivé ou verrouillé : volontairement indiscernables.
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("invalid or expired challenge")]
    InvalidChallenge,
    #[error("invalid TOTP code")]
    InvalidCode,
    #[error("invalid or expired session")]
    InvalidSession,
    #[error("TOTP is already enabled")]
    TotpAlreadyEnabled,
    #[error("TOTP setup has not been started")]
    TotpSetupNotStarted,
    #[error("not found")]
    NotFound,
    /// Règle d'autorisation (anti-escalade, action sur soi-même, rôle système).
    #[error("forbidden: {}", .0.as_str())]
    Forbidden(ErrorCode),
    /// Conflit avec l'état existant (unicité, dernier super-admin).
    #[error("conflict: {}", .0.as_str())]
    Conflict(ErrorCode),
    /// Champ invalide, affiché sous ce champ par le client.
    #[error("invalid {}: {}", .0.field, .0.code.as_str())]
    Invalid(FieldError),
    /// Requête refusée sans champ fautif (modification vide…).
    #[error("unprocessable: {}", .0.as_str())]
    Unprocessable(ErrorCode),
    #[error("unknown permission {0:?}")]
    UnknownPermission(String),
    #[error("permission {0:?} cannot be assigned")]
    PermissionNotAssignable(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for IamError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<PasswordError> for IamError {
    fn from(error: PasswordError) -> Self {
        match error {
            PasswordError::Policy => {
                Self::Invalid(FieldError::new("password", ErrorCode::WeakPassword))
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<TotpError> for IamError {
    fn from(error: TotpError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<SecretBoxError> for IamError {
    fn from(error: SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<kuro_runtime::tls::TlsError> for IamError {
    fn from(error: kuro_runtime::tls::TlsError) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Violation d'unicité traduite en conflit métier.
pub(super) fn unique_violation(error: sqlx::Error, code: ErrorCode) -> IamError {
    match error.as_database_error() {
        Some(db) if db.is_unique_violation() => IamError::Conflict(code),
        _ => error.into(),
    }
}
