//! Configuration du daemon : fichier TOML < variables `KURO_DAEMON_*`.
//!
//! Le fichier est écrit à l'enrôlement ; les sections `api` et `tls` sont obligatoires.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::Duration;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use kuro_runtime::telemetry::LogConfig;
use serde::{Deserialize, Serialize};

/// Emplacement par défaut sur un hôte installé.
pub const DEFAULT_PATH: &str = "/etc/kuro/daemon.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api: ApiConfig,
    pub tls: TlsConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub servers: ServersConfig,
}

/// Serveurs hébergés par le nœud (ADR-0013).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServersConfig {
    /// Données des serveurs : `<data_root>/<server_id>/` (volumes, spécification déployée).
    #[serde(default = "default_data_root")]
    pub data_root: PathBuf,
    /// Image du conteneur utilitaire qui attribue les volumes à l'utilisateur des services.
    #[serde(default = "default_helper_image")]
    pub helper_image: String,
    /// Intervalle entre deux mesures de l'espace disque des serveurs (ADR-0019). Chaque mesure
    /// parcourt tous les volumes : à allonger sur un nœud aux très gros mondes. Une écriture
    /// (fichier, sauvegarde, installation) déclenche de toute façon sa propre mesure.
    #[serde(default = "default_disk_measure_interval")]
    pub disk_measure_interval_secs: u64,
    /// Hors Linux, les fichiers des volumes ne sont pas confinés par `openat2` (ADR-0015) : le
    /// daemon refuse de démarrer, sauf avec ce réglage, réservé au développement.
    #[serde(default)]
    pub allow_insecure_fs: bool,
}

impl ServersConfig {
    /// Bornes de `disk_measure_interval_secs` : assez court pour que le quota reste suivi, assez
    /// long pour qu'une valeur aberrante ne fasse pas tourner le disque en continu.
    pub(crate) const DISK_MEASURE_INTERVAL: RangeInclusive<u64> = 15..=3600;

    pub fn disk_measure_interval(&self) -> Duration {
        Duration::from_secs(self.disk_measure_interval_secs)
    }
}

impl Default for ServersConfig {
    fn default() -> Self {
        Self {
            data_root: default_data_root(),
            helper_image: default_helper_image(),
            disk_measure_interval_secs: default_disk_measure_interval(),
            allow_insecure_fs: false,
        }
    }
}

fn default_data_root() -> PathBuf {
    PathBuf::from("/var/lib/kuro/servers")
}

fn default_helper_image() -> String {
    "docker.io/library/alpine:3.20".to_owned()
}

fn default_disk_measure_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Point d'entrée WebSocket de l'API, obligatoirement en `wss://`.
    pub url: String,
}

/// Matériel mTLS du nœud. La clé privée est générée localement et ne quitte jamais l'hôte.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub ca_cert: PathBuf,
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    Load(#[from] Box<figment::Error>),
    #[error("api.url must use the wss:// scheme (got {0})")]
    InsecureApiUrl(String),
    #[error("servers.disk_measure_interval_secs must be between 15 and 3600 (got {0})")]
    DiskMeasureInterval(u64),
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::from_figment(
            Figment::new()
                .merge(Toml::file_exact(path))
                .merge(Env::prefixed("KURO_DAEMON_").split("__")),
        )
    }

    fn from_figment(figment: Figment) -> Result<Self, ConfigError> {
        let config: Self = figment.extract().map_err(Box::new)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let is_wss = self
            .api
            .url
            .get(..6)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("wss://"));
        if !is_wss {
            return Err(ConfigError::InsecureApiUrl(self.api.url.clone()));
        }
        let interval = self.servers.disk_measure_interval_secs;
        if !ServersConfig::DISK_MEASURE_INTERVAL.contains(&interval) {
            return Err(ConfigError::DiskMeasureInterval(interval));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(servers: &str) -> Result<Config, ConfigError> {
        let toml = format!(
            "[api]
url = \"wss://api.example.test:8443/api/daemon\"
             [tls]
ca_cert = \"ca.pem\"
cert = \"node.pem\"
key = \"node.key\"
             [servers]
{servers}
"
        );
        Config::from_figment(Figment::new().merge(Toml::string(&toml)))
    }

    #[test]
    fn disks_are_measured_every_minute_by_default() {
        let config = parse("").unwrap();
        assert_eq!(
            config.servers.disk_measure_interval(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn the_disk_measure_interval_can_be_lengthened() {
        let config = parse("disk_measure_interval_secs = 900").unwrap();
        assert_eq!(
            config.servers.disk_measure_interval(),
            Duration::from_secs(900)
        );
    }

    #[test]
    fn a_disk_measure_interval_out_of_bounds_is_refused() {
        for interval in [0, 14, 3601] {
            let error = parse(&format!("disk_measure_interval_secs = {interval}")).unwrap_err();
            assert!(
                matches!(error, ConfigError::DiskMeasureInterval(got) if got == interval),
                "{error}"
            );
        }
        for interval in [15, 3600] {
            parse(&format!("disk_measure_interval_secs = {interval}")).unwrap();
        }
    }

    #[test]
    fn an_api_url_without_tls_is_refused() {
        let toml = "[api]
url = \"ws://api.example.test/api/daemon\"
                    [tls]
ca_cert = \"ca.pem\"
cert = \"node.pem\"
key = \"node.key\"
";
        let error = Config::from_figment(Figment::new().merge(Toml::string(toml))).unwrap_err();
        assert!(matches!(error, ConfigError::InsecureApiUrl(_)), "{error}");
    }
}
