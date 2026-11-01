//! Mécanismes d'authentification communs aux espaces admin et client (ADR-0007) :
//! mots de passe, jetons, TOTP, chiffrement des secrets. Aucune notion de compte ni de droit ici.

pub mod jwt;
pub mod keys;
pub mod opaque;
pub mod password;
pub mod recovery_code;
pub mod secret_box;
pub mod totp;
