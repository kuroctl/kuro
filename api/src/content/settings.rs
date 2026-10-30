//! Clés d'API des sources de contenu (ADR-0029), réglées dans le panel : chiffrées avec la clé
//! des secrets des serveurs, vérifiées auprès de la source avant d'être gardées, jamais rendues.

use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::provider::ProviderError;
use super::sources::ContentSources;
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::secret_box;

/// Sources qui demandent une clé d'API.
pub const KEYED_SOURCES: [&str; 2] = ["curseforge", "steam-workshop"];

/// Canal du bus (ADR-0037) : une clé a changé, chaque instance recharge les siennes.
pub const CHANGES_CHANNEL: &str = "kuro_content_sources";

#[derive(Debug, Clone)]
pub struct ContentSourcesState {
    pub pool: PgPool,
    pub key: [u8; 32],
    pub sources: ContentSources,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceKeyError {
    #[error("unknown content source")]
    UnknownSource,
    #[error("api key required")]
    KeyRequired,
    #[error("not configured")]
    NotConfigured,
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl From<secret_box::SecretBoxError> for SourceKeyError {
    fn from(error: secret_box::SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

/// État d'une source à clé, sans la clé.
#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub provider: &'static str,
    pub updated_at: Option<OffsetDateTime>,
}

impl ContentSourcesState {
    fn provider(
        &self,
        name: &str,
    ) -> Result<&std::sync::Arc<dyn super::ContentProvider>, SourceKeyError> {
        match name {
            "curseforge" => Ok(&self.sources.curseforge),
            "steam-workshop" => Ok(&self.sources.steam),
            _ => Err(SourceKeyError::UnknownSource),
        }
    }
}

/// Au démarrage, et à chaque changement fait par une autre instance : les sources reçoivent
/// leur clé enregistrée, et perdent celle qui a été retirée.
pub async fn load(state: &ContentSourcesState) -> Result<(), SourceKeyError> {
    let rows = sqlx::query!("SELECT provider, api_key FROM content_sources")
        .fetch_all(&state.pool)
        .await?;
    for name in KEYED_SOURCES {
        if !rows.iter().any(|row| row.provider == name)
            && let Ok(provider) = state.provider(name)
        {
            provider.set_key(None);
        }
    }
    for row in rows {
        let Ok(provider) = state.provider(&row.provider) else {
            continue;
        };
        let key = secret_box::open(&state.key, &row.api_key)?;
        let key = String::from_utf8(key).map_err(|e| SourceKeyError::Internal(e.to_string()))?;
        provider.set_key(Some(key));
    }
    Ok(())
}

pub async fn status(state: &ContentSourcesState) -> Result<Vec<SourceStatus>, SourceKeyError> {
    let rows = sqlx::query!("SELECT provider, updated_at FROM content_sources")
        .fetch_all(&state.pool)
        .await?;
    Ok(KEYED_SOURCES
        .iter()
        .map(|provider| SourceStatus {
            provider,
            updated_at: rows
                .iter()
                .find(|row| row.provider == *provider)
                .map(|row| row.updated_at),
        })
        .collect())
}

/// Vérifie la clé auprès de la source, puis la garde chiffrée et la donne à la source.
pub async fn set_key(
    state: &ContentSourcesState,
    context: &AuditContext,
    name: &str,
    key: &str,
) -> Result<(), SourceKeyError> {
    let provider = state.provider(name)?;
    let key = key.trim();
    if key.is_empty() {
        return Err(SourceKeyError::KeyRequired);
    }
    provider.verify_key(key).await?;
    let sealed = secret_box::seal(&state.key, key.as_bytes())?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    sqlx::query!(
        "INSERT INTO content_sources (provider, api_key) VALUES ($1, $2)
         ON CONFLICT (provider) DO UPDATE SET api_key = EXCLUDED.api_key, updated_at = now()",
        name,
        &sealed,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::ContentSourceUpdated)
            .target("content_source", name)
            .details(json!({ "provider": name })),
    )
    .await?;
    provider.set_key(Some(key.to_owned()));
    announce(&state.pool, name).await;
    Ok(())
}

/// Les autres instances rechargent leurs clés (ADR-0037) ; un échec n'annule rien, elles le
/// feront à leur prochain redémarrage.
async fn announce(pool: &PgPool, name: &str) {
    if let Err(error) = crate::cluster::Bus::notify(pool, CHANGES_CHANNEL, name).await {
        tracing::warn!(%error, "content source change not announced to other instances");
    }
}

/// Retire la clé : la source n'est plus proposée ; le contenu déjà installé reste.
pub async fn remove_key(
    state: &ContentSourcesState,
    context: &AuditContext,
    name: &str,
) -> Result<(), SourceKeyError> {
    let provider = state.provider(name)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let removed = sqlx::query!("DELETE FROM content_sources WHERE provider = $1", name)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(SourceKeyError::NotConfigured);
    }
    tx.commit(
        AuditEvent::success(AuditAction::ContentSourceRemoved)
            .target("content_source", name)
            .details(json!({ "provider": name })),
    )
    .await?;
    provider.set_key(None);
    announce(&state.pool, name).await;
    Ok(())
}
