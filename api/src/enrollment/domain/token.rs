use std::fmt;

use kuro_runtime::tls::TlsError;

use crate::auth::opaque;

const PREFIX: &str = "kuro_enr_";

/// Token d'enrôlement en clair : affiché une seule fois à l'opérateur, jamais stocké ni journalisé.
/// Préfixé pour être reconnaissable (et détectable par les scanners de secrets).
pub struct EnrollmentToken(String);

impl EnrollmentToken {
    pub fn generate() -> Result<Self, TlsError> {
        Ok(Self(opaque::generate(PREFIX)?))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn hash(&self) -> TokenHash {
        TokenHash::of(&self.0)
    }
}

impl fmt::Debug for EnrollmentToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EnrollmentToken(<redacted>)")
    }
}

/// Seule forme persistée du token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenHash(String);

impl TokenHash {
    pub fn of(token: &str) -> Self {
        Self(opaque::hash(token))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
