//! Configuration de l'API : fichier TOML < variables `KURO_API_*`.
//!
//! Les sections imbriquées se surchargent avec `__`, par exemple `KURO_API_HTTP__BIND`.
//! `database.url`, les chemins PKI de `daemon` et `auth.secret_key` n'ont pas de valeur par défaut.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use kuro_protocol::limits::{HEARTBEAT_INTERVAL, HEARTBEAT_TIMEOUT};
use kuro_runtime::telemetry::LogConfig;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub http: HttpConfig,
    pub database: DatabaseConfig,
    pub daemon: DaemonLinkConfig,
    #[serde(default)]
    pub enrollment: EnrollmentConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub prefabs: PrefabsConfig,
    #[serde(default)]
    pub content: ContentConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub setup: SetupConfig,
    #[serde(default)]
    pub mail: MailConfig,
}

/// Envoi d'e-mails (ADR-0046).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MailConfig {
    /// Accepte un serveur SMTP sans chiffrement (`security = "none"`) : pour une boîte de dev
    /// comme Mailpit, jamais en production. Faux par défaut.
    #[serde(default)]
    pub allow_insecure: bool,
}

/// Première mise en route (ADR-0045).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupConfig {
    /// Fichier du jeton de mise en route, écrit au démarrage tant qu'aucun admin n'existe.
    #[serde(default = "default_setup_token_file")]
    pub token_file: PathBuf,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            token_file: default_setup_token_file(),
        }
    }
}

fn default_setup_token_file() -> PathBuf {
    PathBuf::from("/var/lib/kuro/setup-token")
}

/// Notifications et webhooks (ADR-0044).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotificationsConfig {
    /// URL du panel mise dans chaque notification (lien vers le serveur ou le nœud) ; absente,
    /// `mcp.panel_url` puis `mcp.public_url` servent, sinon pas de lien.
    #[serde(default)]
    pub panel_url: Option<String>,
    /// Webhooks vers des adresses privées acceptés (Slack d'entreprise derrière un VPN…). Faux
    /// par défaut : un webhook ne doit pas servir à joindre le réseau interne de l'API.
    #[serde(default)]
    pub allow_private_targets: bool,
}

/// Serveur MCP et son serveur d'autorisation OAuth (ADR-0023).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    /// URL publique de l'API telle que la voient les clients (`https://panel.example.com`) :
    /// émetteur des jetons, ressource `<url>/mcp`. Absente, le MCP est désactivé.
    #[serde(default)]
    pub public_url: Option<String>,
    /// URL du panel (page de consentement), si elle diffère de `public_url`.
    #[serde(default)]
    pub panel_url: Option<String>,
}

/// Providers de contenu (ADR-0020).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentConfig {
    /// API Modrinth (HTTPS).
    #[serde(default = "default_modrinth_url")]
    pub modrinth_url: String,
    /// API CurseForge (HTTPS) ; la clé d'API se règle dans le panel (ADR-0029).
    #[serde(default = "default_curseforge_url")]
    pub curseforge_url: String,
    /// API Web de Steam (HTTPS), pour le Workshop ; la clé se règle dans le panel.
    #[serde(default = "default_steam_url")]
    pub steam_url: String,
    /// API de SteamGridDB (HTTPS), pour les images du Studio ; la clé se règle dans le panel.
    #[serde(default = "default_steamgriddb_url")]
    pub steamgriddb_url: String,
}

impl Default for ContentConfig {
    fn default() -> Self {
        Self {
            modrinth_url: default_modrinth_url(),
            curseforge_url: default_curseforge_url(),
            steam_url: default_steam_url(),
            steamgriddb_url: default_steamgriddb_url(),
        }
    }
}

fn default_modrinth_url() -> String {
    crate::content::modrinth::DEFAULT_BASE_URL.to_owned()
}

fn default_curseforge_url() -> String {
    crate::content::curseforge::DEFAULT_BASE_URL.to_owned()
}

fn default_steam_url() -> String {
    crate::content::steam::DEFAULT_BASE_URL.to_owned()
}

fn default_steamgriddb_url() -> String {
    crate::content::steamgriddb::DEFAULT_BASE_URL.to_owned()
}

/// Prefabs officiels importés au démarrage (ADR-0011).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefabsConfig {
    /// Un sous-dossier par prefab (`<dir>/<id>/prefab.yaml`).
    #[serde(default = "default_prefabs_dir")]
    pub dir: PathBuf,
    /// Réimporte le dossier dès qu'il change (développement). En production, le bouton
    /// « Recharger » du panel ou un redémarrage suffisent.
    #[serde(default)]
    pub watch: bool,
    /// « Publier » dans le Studio écrit le brouillon dans `dir` (le dossier qui porte déjà cet
    /// identifiant, sinon `<id>/`) et l'importe en `official`, prêt à être commité dans le dépôt
    /// des prefabs (ADR-0042). Développement seulement : en production, `dir` est remplacé à
    /// chaque mise à jour ; publier importe alors en `uploaded`, exporter donne l'archive.
    #[serde(default)]
    pub publish_to_dir: bool,
    /// Stores de prefabs (ADR-0042) : index signé d'un dépôt de prefabs, parcouru dans le panel
    /// et installé à la demande. Par défaut le store officiel `kuroctl/prefabs` ; `stores = []`
    /// pour n'en avoir aucun.
    #[serde(default = "default_stores")]
    pub stores: Vec<StoreConfig>,
}

impl Default for PrefabsConfig {
    fn default() -> Self {
        Self {
            dir: default_prefabs_dir(),
            watch: false,
            publish_to_dir: false,
            stores: default_stores(),
        }
    }
}

/// Un store de prefabs : la racine HTTPS qui sert `index.json`, sa signature
/// `index.json.minisig` et les archives, et la clé publique minisign qui signe l'index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreConfig {
    /// Nom affiché et clé de la source (`store` d'une version importée).
    pub name: String,
    /// Racine du store, en `https://`.
    pub url: String,
    /// Clé publique minisign (base64, telle que dans un fichier `.pub`).
    pub public_key: String,
    /// Jeton d'accès (`Authorization: Bearer`) d'un store privé, facultatif.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Store officiel : le dépôt `kuroctl/prefabs`, publié sur GitHub Pages et signé par la clé des
/// releases de Kuro (ADR-0028).
pub fn default_stores() -> Vec<StoreConfig> {
    vec![StoreConfig {
        name: "official".to_owned(),
        url: "https://kuroctl.github.io/prefabs/".to_owned(),
        public_key: "RWRtWneTRrp1ruOA8XasvWbs6Uch5gWhK3EzHnwQPsbREM/DKJvnWcTY".to_owned(),
        token: None,
    }]
}

fn default_prefabs_dir() -> PathBuf {
    PathBuf::from("prefabs")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    pub bind: SocketAddr,
    /// Origines de page autorisées, en plus de celle de l'API (`https://panel.example.com`) :
    /// WebSocket de console, téléchargements et routes d'authentification vérifient `Origin`.
    /// Vide par défaut : panel servi par l'API ou par un proxy.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// Panel compilé à servir (`panel/dist`), par exemple `/usr/share/kuro/panel`. Absent : l'API
    /// ne sert que `/api` (en dev, le panel passe par Vite).
    #[serde(default)]
    pub panel_dir: Option<std::path::PathBuf>,
    /// Reverse proxys dont `X-Forwarded-For` est cru pour établir l'adresse du client (audit,
    /// limitation de débit). Par défaut les adresses locales : l'API n'écoute qu'en local.
    #[serde(default = "crate::http::policy::default_trusted_proxies")]
    pub trusted_proxies: Vec<IpAddr>,
    /// Requêtes par minute et par adresse cliente sur les routes sans authentification
    /// (connexion, enrôlement, OAuth). `0` : sans limite.
    #[serde(default = "default_rate_limit_per_minute")]
    pub rate_limit_per_minute: u32,
    /// Délai de production d'une réponse, au-delà duquel la requête est abandonnée
    /// (`504 request_timeout`). Les envois de fichiers n'y sont pas soumis. `0` : sans limite.
    #[serde(default = "default_request_timeout")]
    pub request_timeout_secs: u64,
}

impl HttpConfig {
    pub fn request_timeout(&self) -> Option<Duration> {
        (self.request_timeout_secs > 0).then(|| Duration::from_secs(self.request_timeout_secs))
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
            allowed_origins: Vec::new(),
            panel_dir: None,
            trusted_proxies: crate::http::policy::default_trusted_proxies(),
            rate_limit_per_minute: default_rate_limit_per_minute(),
            request_timeout_secs: default_request_timeout(),
        }
    }
}

fn default_rate_limit_per_minute() -> u32 {
    30
}

fn default_request_timeout() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// Connexions gardées ouvertes même au repos (0 : aucune).
    #[serde(default)]
    pub min_connections: u32,
    /// Attente maximale d'une connexion libre du pool.
    #[serde(default = "default_acquire_timeout")]
    pub acquire_timeout_secs: u64,
    /// Connexion inactive fermée au-delà de ce délai (0 : jamais).
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Connexion recyclée au-delà de cet âge (0 : jamais).
    #[serde(default = "default_max_lifetime")]
    pub max_lifetime_secs: u64,
}

impl DatabaseConfig {
    pub fn acquire_timeout(&self) -> Duration {
        Duration::from_secs(self.acquire_timeout_secs)
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        (self.idle_timeout_secs > 0).then(|| Duration::from_secs(self.idle_timeout_secs))
    }

    pub fn max_lifetime(&self) -> Option<Duration> {
        (self.max_lifetime_secs > 0).then(|| Duration::from_secs(self.max_lifetime_secs))
    }
}

/// Listener mTLS dédié aux daemons.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonLinkConfig {
    #[serde(default = "default_daemon_bind")]
    pub bind: SocketAddr,
    /// CA privée : signe les certificats clients des nœuds.
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    /// Certificat serveur présenté aux daemons, signé par la même CA (chaîne incluant la CA).
    pub cert: PathBuf,
    pub key: PathBuf,
    #[serde(default = "default_hello_timeout")]
    pub hello_timeout_secs: u64,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_secs: u64,
    #[serde(default = "default_heartbeat_timeout")]
    pub heartbeat_timeout_secs: u64,
}

impl DaemonLinkConfig {
    pub fn hello_timeout(&self) -> Duration {
        Duration::from_secs(self.hello_timeout_secs)
    }

    pub fn heartbeat_timeout(&self) -> Duration {
        Duration::from_secs(self.heartbeat_timeout_secs)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentConfig {
    /// Durée de validité d'un token d'enrôlement.
    #[serde(default = "default_token_ttl")]
    pub token_ttl_secs: u64,
    /// Durée de validité des certificats clients des nœuds.
    #[serde(default = "default_certificate_validity")]
    pub certificate_validity_days: u32,
}

impl Default for EnrollmentConfig {
    fn default() -> Self {
        Self {
            token_ttl_secs: default_token_ttl(),
            certificate_validity_days: default_certificate_validity(),
        }
    }
}

impl EnrollmentConfig {
    pub fn token_ttl(&self) -> Duration {
        Duration::from_secs(self.token_ttl_secs)
    }

    pub fn certificate_validity(&self) -> time::Duration {
        time::Duration::days(i64::from(self.certificate_validity_days))
    }
}

/// Authentification des humains (ADR-0007).
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 32 octets aléatoires en hexadécimal ; toutes les clés (JWT, TOTP) en sont dérivées.
    pub secret_key: String,
    /// TOTP obligatoire pour ouvrir une session admin. À désactiver uniquement en dev.
    #[serde(default = "default_true")]
    pub require_totp: bool,
    #[serde(default = "default_access_ttl")]
    pub access_ttl_secs: u64,
    #[serde(default = "default_refresh_ttl")]
    pub refresh_ttl_secs: u64,
    /// Durée de vie maximale d'une session admin, rafraîchissements compris : au-delà, il faut
    /// se reconnecter. Au moins `refresh_ttl_secs`.
    #[serde(default = "default_session_max_age")]
    pub session_max_age_secs: u64,
}

/// La clé secrète n'apparaît jamais, même en debug.
impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("secret_key", &"<redacted>")
            .field("require_totp", &self.require_totp)
            .field("access_ttl_secs", &self.access_ttl_secs)
            .field("refresh_ttl_secs", &self.refresh_ttl_secs)
            .field("session_max_age_secs", &self.session_max_age_secs)
            .finish()
    }
}

fn default_true() -> bool {
    true
}

fn default_access_ttl() -> u64 {
    15 * 60
}

fn default_refresh_ttl() -> u64 {
    7 * 24 * 3600
}

fn default_session_max_age() -> u64 {
    30 * 24 * 3600
}

fn default_token_ttl() -> u64 {
    15 * 60
}

fn default_certificate_validity() -> u32 {
    365
}

fn default_max_connections() -> u32 {
    10
}

fn default_acquire_timeout() -> u64 {
    5
}

fn default_idle_timeout() -> u64 {
    10 * 60
}

fn default_max_lifetime() -> u64 {
    30 * 60
}

fn default_daemon_bind() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::UNSPECIFIED, 8443))
}

fn default_hello_timeout() -> u64 {
    10
}

fn default_heartbeat_interval() -> u64 {
    HEARTBEAT_INTERVAL.as_secs()
}

fn default_heartbeat_timeout() -> u64 {
    HEARTBEAT_TIMEOUT.as_secs()
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Load(#[from] Box<figment::Error>),
    /// Tant que TLS n'est pas branché côté HTTP, seule une écoute locale est autorisée.
    #[error("http.bind must be a loopback address while TLS is not configured (got {0})")]
    PlaintextOutsideLoopback(SocketAddr),
    #[error(
        "daemon timings must satisfy 0 < heartbeat_interval_secs < heartbeat_timeout_secs and hello_timeout_secs > 0"
    )]
    InvalidDaemonTimings,
    #[error("enrollment token_ttl_secs and certificate_validity_days must be greater than 0")]
    InvalidEnrollment,
    #[error(
        "database pool must satisfy 1 <= max_connections, min_connections <= max_connections and acquire_timeout_secs > 0"
    )]
    InvalidDatabasePool,
    #[error(transparent)]
    InvalidSecretKey(#[from] crate::auth::keys::InvalidSecretKey),
    #[error(
        "auth ttls must satisfy 0 < access_ttl_secs < refresh_ttl_secs <= session_max_age_secs"
    )]
    InvalidAuthTtl,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut figment = Figment::new();
        if let Some(path) = path {
            figment = figment.merge(Toml::file_exact(path));
        }
        let config: Self = figment
            .merge(Env::prefixed("KURO_API_").split("__"))
            .extract()
            .map_err(Box::new)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if !self.http.bind.ip().is_loopback() {
            return Err(ConfigError::PlaintextOutsideLoopback(self.http.bind));
        }
        let daemon = &self.daemon;
        if daemon.hello_timeout_secs == 0
            || daemon.heartbeat_interval_secs == 0
            || daemon.heartbeat_interval_secs >= daemon.heartbeat_timeout_secs
        {
            return Err(ConfigError::InvalidDaemonTimings);
        }
        if self.enrollment.token_ttl_secs == 0 || self.enrollment.certificate_validity_days == 0 {
            return Err(ConfigError::InvalidEnrollment);
        }
        let database = &self.database;
        if database.max_connections == 0
            || database.min_connections > database.max_connections
            || database.acquire_timeout_secs == 0
        {
            return Err(ConfigError::InvalidDatabasePool);
        }
        crate::auth::keys::AuthKeys::from_hex(&self.auth.secret_key)?;
        if self.auth.access_ttl_secs == 0
            || self.auth.access_ttl_secs >= self.auth.refresh_ttl_secs
            || self.auth.refresh_ttl_secs > self.auth.session_max_age_secs
        {
            return Err(ConfigError::InvalidAuthTtl);
        }
        Ok(())
    }
}
