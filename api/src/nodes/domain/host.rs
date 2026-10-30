/// Caractéristiques de l'hôte annoncées par le daemon dans son `hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeHost {
    pub daemon_version: String,
    pub os: String,
    pub arch: String,
    pub cpu_cores: u32,
    pub memory_bytes: u64,
}
