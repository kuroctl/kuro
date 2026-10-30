use crate::error_code::{ErrorCode, FieldError};

#[derive(Debug, thiserror::Error)]
pub enum NodesError {
    #[error("not found")]
    NotFound,
    #[error("unknown region")]
    UnknownRegion,
    /// Champ invalide, affiché sous ce champ par le client.
    #[error("invalid {}: {}", .0.field, .0.code.as_str())]
    Invalid(FieldError),
    /// Requête refusée sans champ fautif (modification vide…).
    #[error("unprocessable: {}", .0.as_str())]
    Unprocessable(ErrorCode),
    #[error("conflict: {}", .0.as_str())]
    Conflict(ErrorCode),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for NodesError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

/// Violations de contraintes traduites en erreurs métier.
pub(super) fn constraint_violation(error: sqlx::Error, unique: ErrorCode) -> NodesError {
    match error.as_database_error() {
        Some(db) if db.is_unique_violation() => NodesError::Conflict(unique),
        Some(db) if db.is_foreign_key_violation() => {
            NodesError::Conflict(ErrorCode::RegionNotEmpty)
        }
        _ => error.into(),
    }
}
