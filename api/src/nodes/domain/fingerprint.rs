use std::fmt;

/// Empreinte SHA-256 (hexadécimal minuscule) du certificat client d'un nœud.
/// C'est elle, et non un payload, qui identifie le nœud sur le lien daemon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CertFingerprint(String);

impl CertFingerprint {
    pub fn from_der(der: &[u8]) -> Self {
        Self(kuro_runtime::tls::fingerprint(der))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CertFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
