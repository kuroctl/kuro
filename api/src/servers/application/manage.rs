//! Réglages, actions et suppression d'un serveur existant.

use kuro_protocol::ApiMessage;
use kuro_protocol::api::install::{InstallMode, ServerInstall};
use kuro_protocol::api::server::{
    PowerAction, ServerDelete, ServerLimits, ServerPower, ServerSuspend,
};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::create::{BACKUP_LIMIT, CPU_PERCENT, DISK_MB, MEMORY_MB};
use super::deploy::{
    accepted, deploy, disk_bytes, record_failure, resolve_server, resolved, with_steam_account,
};
use super::{
    ServerView, ServersError, ServersState, get, merged_values, seal_secrets, settings_errors,
    split, to_json,
};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::{NodeId, NodeStatus};
use crate::prefabs::application as prefabs;
use crate::servers::domain::{NodeCommands, ServerState};
use crate::servers::infra::{ServerRecord, ServerRepository};
use crate::settings::application::Resources;

/// Modifie des réglages puis redéploie : le daemon recrée les conteneurs et redémarre le
/// serveur s'il tournait. Seules les clés modifiées figurent dans l'audit, jamais les valeurs.
pub async fn update_settings(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    input: &Map<String, Value>,
) -> Result<ServerView, ServersError> {
    // Le prefab est lu avant la transaction : une seule connexion à la fois par requête.
    let known = get(state, id).await?.server;
    let package = prefabs::package(&state.pool, &known.prefab_id, &known.prefab_version).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, id).await? {
        return Err(ServersError::NotFound);
    }
    let server = ServerRepository::get(&mut tx, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    ensure_ready(&server)?;
    if server.prefab_version != known.prefab_version {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    let current = merged_values(state, &server)?;
    let values = kuro_prefab::settings::apply(&package.manifest, Some(&current), input)
        .map_err(settings_errors)?;
    let changed: Vec<&String> = input
        .keys()
        .filter(|key| current.get(*key) != values.get(*key))
        .collect();
    if changed.is_empty() {
        return Err(ServersError::Unprocessable(ErrorCode::EmptyPatch));
    }
    let (public, secrets) = split(&package.manifest, values.clone());
    ServerRepository::update_settings(
        &mut tx,
        id,
        &to_json(&public),
        seal_secrets(state, &secrets)?.as_deref(),
    )
    .await?;
    ServerRepository::set_state(&mut tx, id, ServerState::Deploying.as_str(), None).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerSettingsUpdated)
            .target("server", id)
            .details(json!({ "changed": changed })),
    )
    .await?;

    if let Err(error) = deploy(state, id, false).await {
        record_failure(state, id, &error).await;
        return Err(error);
    }
    get(state, id).await
}

/// Ressources modifiées ; absentes : inchangées.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResourcesPatch {
    pub memory_mb: Option<i32>,
    pub cpu_percent: Option<i32>,
    pub disk_mb: Option<i32>,
    pub backup_limit: Option<i32>,
}

/// Modifie les ressources (ADR-0019). Mémoire, CPU et quota disque s'appliquent à chaud par
/// `server.limits`, envoyé avant l'enregistrement : la base ne diverge pas du nœud. Seules les
/// valeurs modifiées sont soumises aux plafonds du projet.
pub async fn update_resources(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    patch: ResourcesPatch,
) -> Result<ServerView, ServersError> {
    let server = get(state, id).await?.server;
    let current = Resources {
        memory_mb: server.memory_mb,
        cpu_percent: server.cpu_percent,
        disk_mb: server.disk_mb,
        backup_limit: server.backup_limit,
    };
    let next = Resources {
        memory_mb: patch.memory_mb.unwrap_or(current.memory_mb),
        cpu_percent: patch.cpu_percent.unwrap_or(current.cpu_percent),
        disk_mb: patch.disk_mb.unwrap_or(current.disk_mb),
        backup_limit: patch.backup_limit.unwrap_or(current.backup_limit),
    };
    let fields = [
        ("memory_mb", current.memory_mb, next.memory_mb, MEMORY_MB),
        (
            "cpu_percent",
            current.cpu_percent,
            next.cpu_percent,
            CPU_PERCENT,
        ),
        ("disk_mb", current.disk_mb, next.disk_mb, DISK_MB),
        (
            "backup_limit",
            current.backup_limit,
            next.backup_limit,
            BACKUP_LIMIT,
        ),
    ];
    let mut changed = serde_json::Map::new();
    for (field, from, to, range) in &fields {
        if !range.contains(to) {
            return Err(ServersError::Invalid(
                FieldError::new(*field, ErrorCode::InvalidResources)
                    .param("min", *range.start())
                    .param("max", *range.end()),
            ));
        }
        if from != to {
            changed.insert((*field).to_owned(), json!({ "from": from, "to": to }));
        }
    }
    if changed.is_empty() {
        return Err(ServersError::Unprocessable(ErrorCode::EmptyPatch));
    }
    crate::settings::application::get(&state.pool)
        .await
        .map_err(|e| ServersError::Internal(e.to_string()))?
        .check_ceilings(next, |field| changed.contains_key(field))
        .map_err(ServersError::Invalid)?;
    let package = crate::prefabs::application::package(
        &state.pool,
        &server.prefab_id,
        &server.prefab_version,
    )
    .await?;
    crate::settings::application::check_minimums(
        &package.manifest.requires.resources,
        next,
        |field| changed.contains_key(field),
    )
    .map_err(ServersError::Invalid)?;

    let live = ["memory_mb", "cpu_percent", "disk_mb"]
        .iter()
        .any(|field| changed.contains_key(*field));
    if live {
        ensure_ready(&server)?;
        let updated = ServerRecord {
            memory_mb: next.memory_mb,
            cpu_percent: next.cpu_percent,
            disk_mb: next.disk_mb,
            ..server.clone()
        };
        let resolved = resolve_server(state, &updated).await?;
        let limits = ServerLimits {
            server_id: id,
            services: resolved
                .deploy
                .services
                .iter()
                .map(|service| (service.name.clone(), service.limits))
                .collect(),
            disk_bytes: disk_bytes(next.disk_mb),
        };
        accepted(
            state
                .commands
                .send(resolved.node, ApiMessage::ServerLimits(limits))
                .await?,
        )?;
    }
    // Écriture après l'accord du nœud, sans connexion tenue pendant l'attente. Des ressources
    // changées entre-temps par une autre requête sont celles que le nœud applique : refus.
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, id).await? {
        return Err(ServersError::NotFound);
    }
    let latest = ServerRepository::get(&mut tx, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let unchanged = (
        latest.memory_mb,
        latest.cpu_percent,
        latest.disk_mb,
        latest.backup_limit,
    ) == (
        current.memory_mb,
        current.cpu_percent,
        current.disk_mb,
        current.backup_limit,
    );
    if !unchanged {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    ServerRepository::update_resources(
        &mut tx,
        id,
        next.memory_mb,
        next.cpu_percent,
        next.disk_mb,
        next.backup_limit,
    )
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerResourcesUpdated)
            .target("server", id)
            .details(json!({ "changed": changed })),
    )
    .await?;
    get(state, id).await
}

/// Suspend ou réactive le serveur (ADR-0019) ; suspendre l'arrête. Sans effet s'il est déjà
/// dans l'état demandé.
pub async fn set_suspended(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    suspended: bool,
) -> Result<ServerView, ServersError> {
    let server = get(state, id).await?.server;
    if server.suspended_at.is_some() == suspended {
        return get(state, id).await;
    }
    ensure_ready(&server)?;
    accepted(
        state
            .commands
            .send(
                NodeId::from(server.node_id),
                ApiMessage::ServerSuspend(ServerSuspend {
                    server_id: id,
                    suspended,
                }),
            )
            .await?,
    )?;
    // Le nœud a appliqué l'état : la base suit, sans connexion tenue pendant l'attente.
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, id).await? {
        return Err(ServersError::NotFound);
    }
    ServerRepository::set_suspended(&mut tx, id, suspended).await?;
    let action = if suspended {
        AuditAction::ServerSuspended
    } else {
        AuditAction::ServerUnsuspended
    };
    tx.commit(AuditEvent::success(action).target("server", id))
        .await?;
    get(state, id).await
}

pub async fn power(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    action: PowerAction,
) -> Result<(), ServersError> {
    send_power(&state.pool, &*state.commands, context, id, action).await
}

/// Action d'alimentation auditée, aussi envoyée par l'API elle-même (démarrage une fois
/// installé) : seules la base et l'envoi aux nœuds sont nécessaires.
pub(super) async fn send_power(
    pool: &PgPool,
    commands: &dyn NodeCommands,
    context: &AuditContext,
    id: Uuid,
    action: PowerAction,
) -> Result<(), ServersError> {
    let mut conn = pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    drop(conn);
    ensure_ready(&server)?;
    if matches!(action, PowerAction::Start | PowerAction::Restart) {
        ensure_not_suspended(&server)?;
    }
    let result = commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::ServerPower(ServerPower {
                server_id: id,
                action,
            }),
        )
        .await?;
    accepted(result)?;
    // Aucune connexion n'est tenue pendant l'attente du nœud : l'audit s'ouvre une fois accepté.
    let tx = AuditedTx::begin(pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerPower)
            .target("server", id)
            .details(json!({ "action": action_name(action) })),
    )
    .await?;
    Ok(())
}

/// Réinstalle (étapes du prefab sur les données existantes) ou réinitialise (volumes vidés,
/// puis étapes) ; le nœud arrête le serveur d'abord. Acceptée par le nœud, l'état suit
/// (`installing`, puis `offline`).
pub async fn reinstall(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    mode: InstallMode,
) -> Result<(), ServersError> {
    let server = get(state, id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    let resolved = resolved(state, id).await?;
    let install = with_steam_account(state, &resolved.package, resolved.install).await?;
    let result = state
        .commands
        .send(
            resolved.node,
            ApiMessage::ServerInstall(ServerInstall {
                server_id: id,
                mode,
                install,
            }),
        )
        .await?;
    accepted(result)?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerReinstalled)
            .target("server", id)
            .details(json!({ "mode": install_mode_name(mode) })),
    )
    .await?;
    Ok(())
}

fn install_mode_name(mode: InstallMode) -> &'static str {
    match mode {
        InstallMode::Reinstall => "reinstall",
        InstallMode::Reset => "reset",
    }
}

/// Demande la suppression au nœud ; le serveur disparaît quand le nœud la confirme
/// (`server.state` = `deleted`), ce qui libère ses ports.
pub async fn delete(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), ServersError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, id).await? {
        return Err(ServersError::NotFound);
    }
    let server = ServerRepository::get(&mut tx, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    if server.state == ServerState::Deleting.as_str() {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    ensure_node_online(&server)?;
    ServerRepository::set_state(&mut tx, id, ServerState::Deleting.as_str(), None).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerDeleted)
            .target("server", id)
            .details(json!({ "name": server.name, "node_id": server.node_id, "prefab": server.prefab_id })),
    )
    .await?;

    let outcome = state
        .commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::ServerDelete(ServerDelete { server_id: id }),
        )
        .await
        .map_err(ServersError::from)
        .and_then(accepted);
    if let Err(error) = outcome {
        record_failure(state, id, &error).await;
        return Err(error);
    }
    Ok(())
}

/// Une opération est possible : nœud en ligne, serveur ni en déploiement ni en suppression.
pub(super) fn ensure_ready(server: &ServerRecord) -> Result<(), ServersError> {
    if server.state == ServerState::Deploying.as_str()
        || server.state == ServerState::Installing.as_str()
        || server.state == ServerState::Restoring.as_str()
        || server.state == ServerState::Deleting.as_str()
    {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    ensure_node_online(server)
}

/// Un serveur suspendu ne démarre pas et ne touche pas à ses données (ADR-0019).
pub(super) fn ensure_not_suspended(server: &ServerRecord) -> Result<(), ServersError> {
    if server.suspended_at.is_some() {
        return Err(ServersError::Conflict(ErrorCode::ServerSuspended));
    }
    Ok(())
}

/// Quota atteint d'après la dernière mesure du nœud : refus sans l'attendre. Le nœud reste
/// seul juge, la mesure pouvant dater d'une minute.
pub(super) fn ensure_below_quota(server: &ServerRecord) -> Result<(), ServersError> {
    let quota = i64::from(server.disk_mb) * 1024 * 1024;
    if quota > 0 && server.disk_used_bytes.is_some_and(|used| used >= quota) {
        return Err(ServersError::Conflict(ErrorCode::DiskQuotaExceeded));
    }
    Ok(())
}

fn ensure_node_online(server: &ServerRecord) -> Result<(), ServersError> {
    if server.node_status.parse::<NodeStatus>().ok() == Some(NodeStatus::Online) {
        Ok(())
    } else {
        Err(ServersError::Conflict(ErrorCode::NodeOffline))
    }
}

fn action_name(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Start => "start",
        PowerAction::Stop => "stop",
        PowerAction::Restart => "restart",
        PowerAction::Kill => "kill",
    }
}
