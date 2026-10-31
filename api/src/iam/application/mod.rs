//! Cas d'usage du domaine `iam`. Les écritures passent toutes par une
//! [`crate::audit::infra::AuditedTx`] : pas de modification sans événement d'audit.

mod error;
pub mod invitations;
mod login;
pub mod roles;
mod session;
mod tokens;
pub mod users;

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::auth::keys::{AuthKeys, InvalidSecretKey};
use crate::config::AuthConfig;

pub use error::IamError;
pub use login::{
    LoginOutcome, TotpSetup, change_required_password, confirm_totp, login, start_totp_setup,
    verify_totp,
};
pub use roles::{NewRole, RolePatch};
pub use session::{ROTATION_GRACE, Refreshed, authenticate, logout, refresh};
pub use tokens::{AccessToken, Challenge, SessionTokens};
pub use users::{AdminUser, NewAdminUser, Profile, UserPatch};

/// Échecs consécutifs avant verrouillage, et durée du verrouillage.
const MAX_FAILED_ATTEMPTS: i32 = 5;
const LOCK_DURATION: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Copy)]
pub struct AuthSettings {
    pub require_totp: bool,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    /// Durée de vie maximale d'une session, rafraîchissements compris.
    pub session_max_age: Duration,
}

impl From<&AuthConfig> for AuthSettings {
    fn from(config: &AuthConfig) -> Self {
        Self {
            require_totp: config.require_totp,
            access_ttl: Duration::from_secs(config.access_ttl_secs),
            refresh_ttl: Duration::from_secs(config.refresh_ttl_secs),
            session_max_age: Duration::from_secs(config.session_max_age_secs),
        }
    }
}

/// Dépendances des cas d'usage `iam`.
#[derive(Debug, Clone)]
pub struct IamState {
    pub pool: PgPool,
    pub keys: Arc<AuthKeys>,
    pub settings: AuthSettings,
}

impl IamState {
    /// Construction commune au serveur et à la CLI.
    pub fn new(pool: PgPool, config: &AuthConfig) -> Result<Self, InvalidSecretKey> {
        Ok(Self {
            pool,
            keys: Arc::new(AuthKeys::from_hex(&config.secret_key)?),
            settings: AuthSettings::from(config),
        })
    }
}
