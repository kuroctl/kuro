//! Mots de passe : Argon2id (paramètres OWASP par défaut de la crate `argon2`).
//!
//! Le hachage est coûteux par construction : depuis du code async, passer par
//! `tokio::task::spawn_blocking`.

use argon2::{Argon2, PasswordHasher, PasswordVerifier};

pub const MIN_LENGTH: usize = 12;
/// Borne haute : évite qu'un mot de passe géant serve à saturer le CPU.
pub const MAX_LENGTH: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password must be between {MIN_LENGTH} and {MAX_LENGTH} characters")]
    Policy,
    #[error("failed to hash password: {0}")]
    Hash(String),
    #[error(transparent)]
    Random(#[from] kuro_runtime::tls::TlsError),
}

pub fn check_policy(password: &str) -> Result<(), PasswordError> {
    let length = password.chars().count();
    if (MIN_LENGTH..=MAX_LENGTH).contains(&length) {
        Ok(())
    } else {
        Err(PasswordError::Policy)
    }
}

/// Hache un mot de passe conforme à la politique (format PHC).
pub fn hash(password: &str) -> Result<String, PasswordError> {
    check_policy(password)?;
    let salt = kuro_runtime::tls::random_bytes::<16>()?;
    let hash = Argon2::default()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map_err(|error| PasswordError::Hash(error.to_string()))?;
    Ok(hash.to_string())
}

/// `false` aussi pour un hash illisible : on n'ouvre jamais une session sur une erreur.
pub fn verify(password: &str, hash: &str) -> bool {
    password.len() <= MAX_LENGTH * 4
        && Argon2::default()
            .verify_password(password.as_bytes(), hash)
            .is_ok()
}

/// Hash de référence pour les identifiants inconnus : on vérifie quand même, pour que le temps
/// de réponse ne révèle pas l'existence d'un compte.
pub fn dummy_hash() -> &'static str {
    use std::sync::OnceLock;
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash("kuro-dummy-password-for-timing").unwrap_or_default())
}
