use crate::auth::secret_box::SecretBoxError;
use crate::error_code::{ErrorCode, FieldError};
use crate::prefabs::application::PrefabsError;
use crate::servers::domain::CommandFailure;

#[derive(Debug, thiserror::Error)]
pub enum ServersError {
    #[error("not found")]
    NotFound,
    /// Ressource du serveur absente (fichier…), avec son code.
    #[error("missing: {}", .0.as_str())]
    Missing(ErrorCode),
    /// Champ invalide, affiché sous ce champ par le client.
    #[error("invalid {}: {}", .0.field, .0.code.as_str())]
    Invalid(FieldError),
    /// Réglages du prefab invalides (`settings.<clé>`), chacun sous son champ.
    #[error("invalid settings")]
    InvalidSettings(Vec<FieldError>),
    #[error("unprocessable: {}", .0.as_str())]
    Unprocessable(ErrorCode),
    #[error("conflict: {}", .0.as_str())]
    Conflict(ErrorCode),
    /// Permission manquante pour une partie de la demande (anti-escalade, ADR-0007).
    #[error("forbidden: {}", .0.key())]
    Forbidden(crate::iam::domain::Permission),
    /// Le daemon n'a pas accepté la commande.
    #[error("daemon: {code:?}: {message}")]
    Daemon { code: ErrorCode, message: String },
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for ServersError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<PrefabsError> for ServersError {
    fn from(error: PrefabsError) -> Self {
        match error {
            PrefabsError::NotFound => {
                Self::Invalid(FieldError::new("prefab_id", ErrorCode::UnknownPrefab))
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<SecretBoxError> for ServersError {
    fn from(error: SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<CommandFailure> for ServersError {
    fn from(failure: CommandFailure) -> Self {
        match failure {
            CommandFailure::Offline => Self::Conflict(ErrorCode::NodeOffline),
            CommandFailure::Timeout => Self::Daemon {
                code: ErrorCode::DaemonTimeout,
                message: failure.to_string(),
            },
            CommandFailure::Closed | CommandFailure::Unexpected => Self::Daemon {
                code: ErrorCode::DaemonError,
                message: failure.to_string(),
            },
        }
    }
}
