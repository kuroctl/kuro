//! Création d'un serveur : validation, réglages, secrets générés, allocations, puis déploiement.

use std::collections::BTreeMap;

use kuro_runtime::tls;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{
    ServerView, ServersError, ServersState, deploy, get, seal_secrets, settings_errors, split,
    to_json,
};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::NodeStatus;
use crate::nodes::infra::{AllocationRepository, NodeRepository};
use crate::prefabs::application::{self as prefabs, PrefabsError};
use crate::servers::domain::ServerEvent;
use crate::servers::infra::{NewServerRow, ServerRepository};
use crate::settings::application::Resources;

pub const MEMORY_MB: std::ops::RangeInclusive<i32> = 64..=1_048_576;
pub const CPU_PERCENT: std::ops::RangeInclusive<i32> = 1..=12_800;
pub const DISK_MB: std::ops::RangeInclusive<i32> = 0..=104_857_600;
pub const BACKUP_LIMIT: std::ops::RangeInclusive<i32> = 0..=100;

#[derive(Debug, Clone)]
pub struct NewServer {
    pub name: String,
    pub node_id: Uuid,
    pub prefab_id: String,
    /// Dernière version importée si absente.
    pub prefab_version: Option<String>,
    /// Ressources : valeur par défaut des paramètres du projet si absentes (ADR-0019).
    pub memory_mb: Option<i32>,
    pub cpu_percent: Option<i32>,
    pub disk_mb: Option<i32>,
    pub settings: Map<String, Value>,
    /// Port public du prefab → allocation choisie ; sinon la première libre du nœud.
    pub allocations: BTreeMap<String, Uuid>,
    /// Démarrer le serveur une fois déployé et installé.
    pub start_when_installed: bool,
    /// Sauvegardes gardées (ADR-0018).
    pub backup_limit: Option<i32>,
    /// Propriétaire (ADR-0051) ; `None` : serveur interne.
    pub owner_ref: Option<String>,
}

pub async fn create(
    state: &ServersState,
    context: &AuditContext,
    input: NewServer,
) -> Result<ServerView, ServersError> {
    let name = input.name.trim();
    if !crate::validation::is_display_text(name, 1, 64) {
        return Err(ServersError::Invalid(FieldError::new(
            "name",
            ErrorCode::InvalidServerName,
        )));
    }
    let platform = crate::settings::application::get(&state.pool)
        .await
        .map_err(|e| ServersError::Internal(e.to_string()))?;
    let resources = Resources {
        memory_mb: input.memory_mb.unwrap_or(platform.default_memory_mb),
        cpu_percent: input.cpu_percent.unwrap_or(platform.default_cpu_percent),
        disk_mb: input.disk_mb.unwrap_or(platform.default_disk_mb),
        backup_limit: input.backup_limit.unwrap_or(platform.default_backup_limit),
    };
    for (field, value, range) in [
        ("memory_mb", resources.memory_mb, MEMORY_MB),
        ("cpu_percent", resources.cpu_percent, CPU_PERCENT),
        ("disk_mb", resources.disk_mb, DISK_MB),
        ("backup_limit", resources.backup_limit, BACKUP_LIMIT),
    ] {
        if !range.contains(&value) {
            return Err(ServersError::Invalid(
                FieldError::new(field, ErrorCode::InvalidResources)
                    .param("min", *range.start())
                    .param("max", *range.end()),
            ));
        }
    }

    let owner_ref = super::owner::checked(input.owner_ref.clone())?;

    platform
        .check_ceilings(resources, |_| true)
        .map_err(ServersError::Invalid)?;

    let version = match &input.prefab_version {
        Some(version) => version.clone(),
        None => latest_version(state, &input.prefab_id).await?,
    };
    let package = prefabs::package(&state.pool, &input.prefab_id, &version).await?;
    // Brouillon : seulement sur un nœud de test, vérifié avec le nœud ci-dessous (ADR-0011).
    let draft = prefabs::is_draft(&package.manifest);
    // Refusé avant tout déploiement : sans compte, l'installation échouerait sur le nœud.
    if package.manifest.needs_steam_account()
        && !super::steam_account::configured(&state.pool).await?
    {
        return Err(ServersError::Unprocessable(ErrorCode::SteamAccountRequired));
    }
    crate::settings::application::check_minimums(
        &package.manifest.requires.resources,
        resources,
        |_| true,
    )
    .map_err(ServersError::Invalid)?;

    let mut values = kuro_prefab::settings::apply(&package.manifest, None, &input.settings)
        .map_err(settings_errors)?;
    for (key, length) in kuro_prefab::settings::secrets_to_generate(&package.manifest) {
        if let std::collections::btree_map::Entry::Vacant(e) = values.entry(key) {
            e.insert(Value::String(generate_secret(length)?));
        }
    }
    let (public, secrets) = split(&package.manifest, values);
    let sealed = seal_secrets(state, &secrets)?;
    // Site web (ADR-0038) : un identifiant sous le domaine des sites, exigé par le prefab.
    let web_id = match package.manifest.web_port() {
        Some(_) => Some(super::web::new_web_id(&state.pool).await?),
        None => None,
    };

    let public_ports: Vec<String> = package
        .manifest
        .services
        .values()
        .flat_map(|s| {
            s.ports
                .iter()
                .filter(|(_, p)| p.public)
                .map(|(n, _)| n.clone())
        })
        .collect();
    if let Some(unknown) = input
        .allocations
        .keys()
        .find(|port| !public_ports.contains(port))
    {
        return Err(ServersError::Invalid(
            FieldError::new("allocations", ErrorCode::UnknownAllocation)
                .param("port", unknown.as_str()),
        ));
    }

    let id = Uuid::now_v7();
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let node = NodeRepository::get(&mut tx, input.node_id)
        .await?
        .ok_or_else(|| ServersError::Invalid(FieldError::new("node_id", ErrorCode::NotFound)))?;
    if node.status.parse::<NodeStatus>().ok() != Some(NodeStatus::Online) {
        return Err(ServersError::Conflict(ErrorCode::NodeOffline));
    }
    if draft && !node.trials {
        return Err(ServersError::Invalid(FieldError::new(
            "node_id",
            ErrorCode::PrefabDraftNeedsTestNode,
        )));
    }
    ServerRepository::insert(
        &mut tx,
        NewServerRow {
            id,
            name,
            node_id: input.node_id,
            prefab_id: &input.prefab_id,
            prefab_version: &version,
            settings: &to_json(&public),
            secrets: sealed.as_deref(),
            memory_mb: resources.memory_mb,
            cpu_percent: resources.cpu_percent,
            disk_mb: resources.disk_mb,
            start_when_installed: input.start_when_installed,
            backup_limit: resources.backup_limit,
            web_id: web_id.as_deref(),
            owner_ref: owner_ref.as_deref(),
        },
    )
    .await?;

    let mut assigned = Vec::new();
    let mut ports = BTreeMap::new();
    for port in &public_ports {
        let allocation = match input.allocations.get(port) {
            Some(&allocation_id) => {
                AllocationRepository::lock_free(&mut tx, input.node_id, allocation_id)
                    .await?
                    .map(|(ip, number)| (allocation_id, ip, number))
                    .ok_or_else(|| {
                        ServersError::Invalid(
                            FieldError::new("allocations", ErrorCode::AllocationTaken)
                                .param("port", port.as_str()),
                        )
                    })?
            }
            None => AllocationRepository::lock_first_free(&mut tx, input.node_id, &assigned)
                .await?
                .ok_or(ServersError::Conflict(ErrorCode::NoFreeAllocation))?,
        };
        AllocationRepository::assign(&mut tx, allocation.0, id, port).await?;
        assigned.push(allocation.0);
        ports.insert(port.clone(), allocation.2);
    }

    tx.commit(
        AuditEvent::success(AuditAction::ServerCreated)
            .target("server", id)
            .details(json!({
                "name": name,
                "node_id": input.node_id,
                "prefab": input.prefab_id,
                "version": version,
                "memory_mb": resources.memory_mb,
                "cpu_percent": resources.cpu_percent,
                "disk_mb": resources.disk_mb,
                "ports": ports,
                "web_id": web_id,
                "start_when_installed": input.start_when_installed,
                "backup_limit": resources.backup_limit,
                "owner_ref": owner_ref,
            })),
    )
    .await?;

    state.events.emit(ServerEvent::Created {
        server_id: id,
        owner_ref: owner_ref.clone(),
    });
    // Le serveur existe : un échec de déploiement le laisse en `failed`, avec sa raison.
    deploy::deploy_recording_failure(state, id, true).await;
    get(state, id).await
}

async fn latest_version(state: &ServersState, prefab_id: &str) -> Result<String, ServersError> {
    prefabs::list(&state.pool)
        .await?
        .into_iter()
        .filter(|record| record.id == prefab_id)
        .filter_map(|record| semver::Version::parse(&record.version).ok())
        .max()
        .map(|version| version.to_string())
        .ok_or(ServersError::from(PrefabsError::NotFound))
}

/// Secret aléatoire lisible (alphanumérique), généré par le CSPRNG de l'hôte.
pub(super) fn generate_secret(length: usize) -> Result<String, ServersError> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut out = String::with_capacity(length);
    while out.len() < length {
        let bytes = tls::random_bytes::<64>().map_err(|e| ServersError::Internal(e.to_string()))?;
        // Rejet des octets au-delà du plus grand multiple de la taille de l'alphabet : pas de biais.
        let limit = 256 - (256 % ALPHABET.len());
        for byte in bytes {
            if usize::from(byte) < limit && out.len() < length {
                out.push(char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]));
            }
        }
    }
    Ok(out)
}
