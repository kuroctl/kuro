/// Caractéristiques de l'hôte annoncées par le daemon dans son `hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeHost {
    pub daemon_version: String,
    pub os: String,
    pub arch: String,
    pub cpu_cores: u32,
    pub memory_bytes: u64,
    /// Port SFTP et empreinte de la clé d'hôte (ADR-0047) ; absent : le nœud ne sert pas le SFTP.
    pub sftp: Option<(u16, String)>,
    /// Adresse du pair TCP du lien, vue par l'API (repli pour joindre le nœud, ADR-0047).
    pub link_address: Option<String>,
}
