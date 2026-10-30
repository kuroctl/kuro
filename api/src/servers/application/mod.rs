//! Cas d'usage du domaine `servers` (ADR-0013).
//!
//! Les réglages `secret` (mots de passe générés des prefabs) sont séparés des autres, chiffrés
//! au repos et jamais renvoyés : ils ne rejoignent les réglages qu'au moment de résoudre la
//! spécification envoyée au daemon.

pub mod backups;
pub mod bulk;
mod console;
pub mod content;
mod create;
mod deploy;
mod error;
pub mod files;
mod link;
mod manage;
pub mod scheduler;
pub mod schedules;
pub mod search;
pub mod steam_account;
pub mod tickets;
pub mod upgrade;
pub mod web;

use std::sync::Arc;

use kuro_prefab::Manifest;
use kuro_prefab::manifest::FieldType;
use kuro_prefab::settings::{Reason, SettingError, Values};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

pub use console::{ConsoleAccess, ConsoleEvent, ConsoleHub, Viewer, command, console_ticket};
pub use create::{BACKUP_LIMIT, CPU_PERCENT, DISK_MB, MEMORY_MB, NewServer, create};
pub use error::ServersError;
pub use link::LinkEvents;
pub use manage::{
    ResourcesPatch, delete, power, reinstall, set_suspended, update_resources, update_settings,
};
pub use tickets::TICKET_TTL;

use crate::auth::secret_box;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::infra::AllocationRepository;
use crate::servers::domain::NodeCommands;
use crate::servers::infra::{ServerRecord, ServerRepository};

/// Dépendances des cas d'usage `servers`.
#[derive(Debug, Clone)]
pub struct ServersState {
    pub pool: PgPool,
    /// Envoi des commandes aux nœuds connectés (registre du lien daemon).
    pub commands: Arc<dyn NodeCommands>,
    /// Clé de chiffrement des réglages `secret`.
    pub secrets_key: [u8; 32],
    /// Spectateurs des consoles et tickets du WebSocket navigateur.
    pub console: ConsoleHub,
    pub console_access: ConsoleAccess,
    /// Sources des mods, plugins et modpacks (ADR-0020, ADR-0029).
    pub content: crate::content::ContentSources,
}

/// Serveur et ses ports.
#[derive(Debug, Clone)]
pub struct ServerView {
    pub server: ServerRecord,
    /// Changements de contenu en attente du prochain démarrage.
    pub pending: crate::servers::infra::PendingChanges,
    /// (port du prefab, adresse, port de l'hôte).
    pub allocations: Vec<(String, String, i32)>,
}

/// Serveurs par nom, `limit` au plus (`None` : tous) à partir de `offset`, en trois requêtes
/// quelle que soit la taille de la page.
pub async fn list(
    state: &ServersState,
    limit: Option<usize>,
    offset: usize,
) -> Result<Vec<ServerView>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let servers = ServerRepository::list(
        &mut conn,
        limit.map(|limit| i64::try_from(limit).unwrap_or(i64::MAX)),
        i64::try_from(offset).unwrap_or(i64::MAX),
    )
    .await?;
    views_of(&mut conn, servers).await
}

/// Vues de serveurs déjà lus : ports et changements en attente chargés en une requête chacun.
pub(super) async fn views_of(
    conn: &mut sqlx::PgConnection,
    servers: Vec<ServerRecord>,
) -> Result<Vec<ServerView>, ServersError> {
    let ids: Vec<Uuid> = servers.iter().map(|s| s.id).collect();
    let pending = ServerRepository::pending(conn, &ids).await?;
    let mut allocations = AllocationRepository::of_servers(conn, &ids).await?;
    Ok(servers
        .into_iter()
        .map(|server| ServerView {
            pending: pending
                .iter()
                .find(|p| p.server_id == server.id)
                .copied()
                .unwrap_or_default(),
            allocations: allocations.remove(&server.id).unwrap_or_default(),
            server,
        })
        .collect())
}

pub async fn get(state: &ServersState, id: Uuid) -> Result<ServerView, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let allocations = AllocationRepository::of_server(&mut conn, id).await?;
    let pending = ServerRepository::pending(&mut conn, &[id])
        .await?
        .into_iter()
        .next()
        .unwrap_or_default();
    Ok(ServerView {
        server,
        pending,
        allocations,
    })
}

/// Clés des secrets définis (jamais leurs valeurs).
pub fn secret_keys(state: &ServersState, server: &ServerRecord) -> Vec<String> {
    open_secrets(state, server)
        .map(|s| s.into_keys().collect())
        .unwrap_or_default()
}

/// Sépare les réglages publics des secrets, selon le type des champs du prefab.
fn split(manifest: &Manifest, values: Values) -> (Values, Values) {
    values.into_iter().partition(|(key, _)| {
        manifest
            .settings
            .field(key)
            .is_none_or(|field| field.kind != FieldType::Secret)
    })
}

fn seal_secrets(state: &ServersState, secrets: &Values) -> Result<Option<Vec<u8>>, ServersError> {
    if secrets.is_empty() {
        return Ok(None);
    }
    let plain = serde_json::to_vec(secrets).map_err(|e| ServersError::Internal(e.to_string()))?;
    Ok(Some(secret_box::seal(&state.secrets_key, &plain)?))
}

fn open_secrets(state: &ServersState, server: &ServerRecord) -> Result<Values, ServersError> {
    match &server.secrets {
        Some(sealed) => {
            let plain = secret_box::open(&state.secrets_key, sealed)?;
            serde_json::from_slice(&plain).map_err(|e| ServersError::Internal(e.to_string()))
        }
        None => Ok(Values::new()),
    }
}

/// Réglages complets (publics + secrets) d'un serveur stocké.
fn merged_values(state: &ServersState, server: &ServerRecord) -> Result<Values, ServersError> {
    let mut values: Values = serde_json::from_value(server.settings.clone())
        .map_err(|e| ServersError::Internal(e.to_string()))?;
    values.extend(open_secrets(state, server)?);
    Ok(values)
}

/// Erreurs de réglages, chacune sous `settings.<clé>`.
fn settings_errors(errors: Vec<SettingError>) -> ServersError {
    ServersError::InvalidSettings(
        errors
            .into_iter()
            .map(|e| {
                let code = if e.reason == Reason::NotEditable {
                    ErrorCode::SettingNotEditable
                } else {
                    ErrorCode::InvalidSetting
                };
                FieldError::new(format!("settings.{}", e.field), code)
                    .param("reason", e.reason.as_str())
            })
            .collect(),
    )
}

/// Valeur JSON des réglages publics, pour la base et l'API.
fn to_json(values: &Values) -> Value {
    Value::Object(values.clone().into_iter().collect())
}
