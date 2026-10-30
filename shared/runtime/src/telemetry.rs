//! Initialisation des logs `tracing` : lisible en dev, JSON en prod.

use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

/// Section `[log]` des fichiers de configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    pub format: LogFormat,
    /// Directive `EnvFilter`, par exemple `info,kuro_api=debug`.
    pub filter: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Pretty,
    Json,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::Pretty,
            filter: "info".to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("invalid log filter: {0}")]
    Filter(#[from] tracing_subscriber::filter::ParseError),
    #[error("failed to install tracing subscriber: {0}")]
    Init(Box<dyn std::error::Error + Send + Sync>),
}

/// Installe le subscriber global. À appeler une seule fois, au démarrage.
pub fn init(config: &LogConfig) -> Result<(), TelemetryError> {
    let filter = EnvFilter::try_new(&config.filter)?;
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match config.format {
        LogFormat::Json => builder.json().try_init(),
        LogFormat::Pretty => builder.try_init(),
    }
    .map_err(TelemetryError::Init)
}
