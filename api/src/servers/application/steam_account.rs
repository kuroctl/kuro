//! Compte Steam de l'installation (ADR-0033), pour les serveurs que SteamCMD refuse en anonyme.
//! Le mot de passe est chiffré avec la clé des secrets des serveurs, jamais renvoyé ni journalisé ;
//! il ne part qu'avec une installation dont une étape `steamcmd` le demande.

use kuro_protocol::api::SteamLogin;
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{ServersError, ServersState};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::secret_box;
use crate::error_code::{ErrorCode, FieldError};

/// Portée du seul compte réglable pour l'instant.
const INSTALLATION: &str = "installation";
/// Longueur maximale d'un mot de passe Steam.
const MAX_PASSWORD_CHARS: usize = 64;

/// État du compte, sans son mot de passe.
#[derive(Debug, Clone)]
pub struct SteamAccountStatus {
    pub username: String,
    pub updated_at: OffsetDateTime,
}

pub async fn status(state: &ServersState) -> Result<Option<SteamAccountStatus>, ServersError> {
    let row = sqlx::query!(
        "SELECT username, updated_at FROM steam_accounts WHERE scope = $1",
        INSTALLATION
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|row| SteamAccountStatus {
        username: row.username,
        updated_at: row.updated_at,
    }))
}

/// Identifiant d'un compte Steam : lettres, chiffres et `_`, comme Steam les accepte.
fn checked_username(username: &str) -> Result<&str, ServersError> {
    let username = username.trim();
    let valid = (3..=64).contains(&username.len())
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(ServersError::Invalid(FieldError::new(
            "username",
            ErrorCode::SteamAccountUsernameInvalid,
        )));
    }
    Ok(username)
}

/// Le mot de passe finit dans un script SteamCMD entre guillemets : ni guillemet, ni barre
/// oblique inverse, ni caractère de contrôle.
fn checked_password(password: &str) -> Result<&str, ServersError> {
    let valid = !password.is_empty()
        && password.chars().count() <= MAX_PASSWORD_CHARS
        && !password
            .chars()
            .any(|c| c.is_control() || c == '"' || c == '\\');
    if !valid {
        return Err(ServersError::Invalid(FieldError::new(
            "password",
            ErrorCode::SteamAccountPasswordInvalid,
        )));
    }
    Ok(password)
}

/// Enregistre (ou remplace) le compte. Steam n'offre pas de vérification hors de SteamCMD : la
/// première installation qui s'en sert la fait, et son échec est explicite.
pub async fn set(
    state: &ServersState,
    context: &AuditContext,
    username: &str,
    password: &str,
) -> Result<(), ServersError> {
    let username = checked_username(username)?;
    let password = checked_password(password)?;
    let sealed = secret_box::seal(&state.secrets_key, password.as_bytes())?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    sqlx::query!(
        "INSERT INTO steam_accounts (id, scope, username, password) VALUES ($1, $2, $3, $4)
         ON CONFLICT (scope) WHERE scope = 'installation'
         DO UPDATE SET username = EXCLUDED.username, password = EXCLUDED.password,
                       updated_at = now()",
        Uuid::now_v7(),
        INSTALLATION,
        username,
        &sealed,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::SteamAccountUpdated)
            .target("steam_account", INSTALLATION)
            .details(json!({ "scope": INSTALLATION, "username": username })),
    )
    .await?;
    Ok(())
}

/// Retire le compte : les prefabs qui l'exigent ne s'installent plus ; les serveurs installés
/// restent.
pub async fn remove(state: &ServersState, context: &AuditContext) -> Result<(), ServersError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let removed = sqlx::query_scalar!(
        "DELETE FROM steam_accounts WHERE scope = $1 RETURNING username",
        INSTALLATION
    )
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ServersError::Missing(ErrorCode::SteamAccountNotConfigured))?;
    tx.commit(
        AuditEvent::success(AuditAction::SteamAccountRemoved)
            .target("steam_account", INSTALLATION)
            .details(json!({ "scope": INSTALLATION, "username": removed })),
    )
    .await?;
    Ok(())
}

/// Compte déchiffré, pour l'envoi d'une installation seulement.
pub(super) async fn login(state: &ServersState) -> Result<Option<SteamLogin>, ServersError> {
    let row = sqlx::query!(
        "SELECT username, password FROM steam_accounts WHERE scope = $1",
        INSTALLATION
    )
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let password = secret_box::open(&state.secrets_key, &row.password)?;
    let password =
        String::from_utf8(password).map_err(|e| ServersError::Internal(e.to_string()))?;
    Ok(Some(SteamLogin {
        username: row.username,
        password,
    }))
}

/// Un compte est réglé (pour savoir si un prefab qui l'exige est déployable).
pub async fn configured(pool: &sqlx::PgPool) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM steam_accounts WHERE scope = $1) AS \"exists!\"",
        INSTALLATION
    )
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_is_letters_digits_and_underscores() {
        assert!(checked_username(" kuro_servers2 ").is_ok());
        for bad in ["ab", "kuro servers", "kuro-servers", "kuro\"x", ""] {
            assert!(checked_username(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_password_cannot_break_out_of_the_steamcmd_script() {
        assert!(checked_password("Pa ss!#$%&'()*+,-./:;<=>?@[]^_`{|}~").is_ok());
        for bad in ["", "a\"b", "a\\b", "a\nlogin x", "a\tb", &"x".repeat(65)] {
            assert!(checked_password(bad).is_err(), "{bad:?}");
        }
    }
}
