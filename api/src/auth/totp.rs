//! TOTP (RFC 6238) : SHA-1, 6 chiffres, pas de 30 s, tolérance d'un pas de part et d'autre.
//! Paramètres imposés par la compatibilité avec les applications d'authentification courantes.

use totp_rs::{Algorithm, Builder, Secret, Totp};

/// Secret de 160 bits (taille recommandée par la RFC 4226 pour SHA-1).
pub const SECRET_BYTES: usize = 20;
const ISSUER: &str = "Kuro";

#[derive(Debug, thiserror::Error)]
pub enum TotpError {
    #[error("invalid TOTP secret")]
    InvalidSecret,
    #[error(transparent)]
    Random(#[from] kuro_runtime::tls::TlsError),
}

pub fn generate_secret() -> Result<[u8; SECRET_BYTES], TotpError> {
    Ok(kuro_runtime::tls::random_bytes::<SECRET_BYTES>()?)
}

/// Secret en base32, à saisir à la main si le QR code n'est pas lisible.
pub fn secret_base32(secret: &[u8]) -> String {
    Secret::from(secret.to_vec()).to_base32()
}

/// URI `otpauth://` à transformer en QR code côté panel.
pub fn otpauth_uri(secret: &[u8], account: &str) -> Result<String, TotpError> {
    totp(secret, account)?
        .to_url()
        .map_err(|_| TotpError::InvalidSecret)
}

/// Pas de temps du code s'il est valide maintenant **et** postérieur à `last_step` (anti-rejeu).
pub fn verify(
    secret: &[u8],
    code: &str,
    unix_time: u64,
    last_step: Option<u64>,
) -> Result<Option<u64>, TotpError> {
    let step = totp(secret, "")?.check(code.trim(), unix_time);
    Ok(step.filter(|step| last_step.is_none_or(|last| *step > last)))
}

/// Code courant, pour les tests et l'outillage.
pub fn current_code(secret: &[u8], unix_time: u64) -> Result<String, TotpError> {
    Ok(totp(secret, "")?.generate(unix_time).to_string())
}

fn totp(secret: &[u8], account: &str) -> Result<Totp, TotpError> {
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(secret.to_vec())
        .with_issuer(Some(ISSUER))
        .with_account_name(account)
        .build()
        .map_err(|_| TotpError::InvalidSecret)
}
