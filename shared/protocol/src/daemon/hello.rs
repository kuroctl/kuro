use serde::{Deserialize, Serialize};

/// Premier message après l'ouverture du lien. L'API répond `hello.ack` ou `error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Voir [`crate::version`].
    pub protocol_version: u16,
    /// Version du binaire `kuro-daemon`.
    pub daemon_version: String,
    pub host: HostInfo,
    /// Accès SFTP du nœud (ADR-0047), absent s'il n'est pas configuré.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sftp: Option<SftpEndpoint>,
}

/// Port SFTP du nœud et empreinte de sa clé d'hôte, montrées dans le panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SftpEndpoint {
    pub port: u16,
    /// `SHA256:…`, comme l'affichent OpenSSH, FileZilla et WinSCP.
    pub host_key_sha256: String,
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
