use serde::{Deserialize, Serialize};

/// Premier message après l'ouverture du lien. L'API répond `hello.ack` ou `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Voir [`crate::version`].
    pub protocol_version: u16,
    /// Version du binaire `kuro-daemon`.
    pub daemon_version: String,
    pub host: HostInfo,
}

/// Capacité de l'hôte, utilisée pour le placement des serveurs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    /// Valeur de `std::env::consts::OS`.
    pub os: String,
    /// Valeur de `std::env::consts::ARCH`.
    pub arch: String,
    pub cpu_cores: u32,
    pub memory_bytes: u64,
}
