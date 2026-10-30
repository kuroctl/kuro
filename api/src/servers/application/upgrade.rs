//! Mise à jour d'un serveur vers une version plus récente de son prefab (ADR-0024) : aperçu
//! calculé sans rien écrire, puis sauvegarde attendue, réglages migrés, ports ajustés et
//! redéploiement (installation relancée si ses étapes ont changé).

use std::collections::BTreeSet;

use kuro_prefab::Manifest;
use kuro_prefab::settings::{SettingError, migrate};
use serde_json::{Value, json};
use uuid::Uuid;

use super::deploy::{deploy, record_failure};
use super::manage::{ensure_not_suspended, ensure_ready};
use super::{
    ServersError, ServersState, backups, get, merged_values, scheduler, seal_secrets, split,
    to_json,
};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::ErrorCode;
use crate::nodes::infra::AllocationRepository;
use crate::prefabs::application::{self as prefabs, PrefabsError};
use crate::servers::domain::ServerState;
use crate::servers::infra::{ServerRecord, ServerRepository};

/// Ce qui empêche la mise à jour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Blocker {
    /// Valeur devenue invalide dans la nouvelle version (`reason` : raison de validation).
    Setting { field: String, reason: &'static str },
    /// Emplacement de contenu disparu alors qu'il contient des mods.
    ContentSlot { slot: String },
    /// Pas assez de ports libres sur le nœud pour les nouveaux ports publics.
    NoFreePort { needed: usize, free: i64 },
    /// La nouvelle version est un brouillon et le serveur n'est pas sur un nœud de test.
    NotDeployable,
    /// Ressource sous le minimum de la nouvelle version (ADR-0031) : à remonter d'abord.
    ResourceBelowMinimum {
        resource: &'static str,
        minimum: u32,
        current: i32,
    },
}

/// Image d'un service avant et après.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageChange {
    pub service: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub from: String,
    pub to: String,
    pub renamed: Vec<(String, String)>,
    pub removed: Vec<String>,
    pub added: Vec<String>,
    pub ports_added: Vec<String>,
    pub ports_removed: Vec<String>,
    pub images: Vec<ImageChange>,
    /// Les étapes d'installation ont changé : elles seront relancées, données gardées.
    pub reinstall: bool,
    pub blockers: Vec<Blocker>,
}

impl Preview {
    pub fn blocked(&self) -> bool {
        !self.blockers.is_empty()
    }
}

fn parse(version: &str) -> Option<semver::Version> {
    version.parse().ok()
}

/// Versions plus récentes du prefab du serveur, de la plus ancienne à la plus récente.
pub async fn newer_versions(
    state: &ServersState,
    server: &ServerRecord,
) -> Result<Vec<String>, ServersError> {
    let current = parse(&server.prefab_version);
    let mut versions: Vec<(semver::Version, String)> = prefabs::list(&state.pool)
        .await?
        .into_iter()
        .filter(|p| p.id == server.prefab_id)
        .filter_map(|p| parse(&p.version).map(|v| (v, p.version)))
        .filter(|(v, _)| current.as_ref().is_none_or(|current| v > current))
        .collect();
    versions.sort();
    Ok(versions.into_iter().map(|(_, text)| text).collect())
}

/// Ports publics d'un manifest.
fn public_ports(manifest: &Manifest) -> BTreeSet<String> {
    manifest
        .services
        .values()
        .flat_map(|s| {
            s.ports
                .iter()
                .filter(|(_, p)| p.public)
                .map(|(n, _)| n.clone())
        })
        .collect()
}

fn setting_blockers(errors: Vec<SettingError>) -> Vec<Blocker> {
    errors
        .into_iter()
        .map(|e| Blocker::Setting {
            field: e.field,
            reason: e.reason.as_str(),
        })
        .collect()
}

/// Aperçu de la mise à jour vers `to`, sans rien écrire.
pub async fn preview(state: &ServersState, id: Uuid, to: &str) -> Result<Preview, ServersError> {
    let server = get(state, id).await?.server;
    preview_of(state, &server, to).await
}

async fn preview_of(
    state: &ServersState,
    server: &ServerRecord,
    to: &str,
) -> Result<Preview, ServersError> {
    let from = parse(&server.prefab_version);
    let target = parse(to);
    let newer = matches!((&from, &target), (Some(from), Some(to)) if to > from);
    if !newer {
        return Err(ServersError::Unprocessable(ErrorCode::UpgradeNotNewer));
    }
    let old = prefabs::package(&state.pool, &server.prefab_id, &server.prefab_version).await?;
    let new = match prefabs::package(&state.pool, &server.prefab_id, to).await {
        Err(PrefabsError::NotFound) => {
            return Err(ServersError::Unprocessable(ErrorCode::UpgradeNotNewer));
        }
        other => other?,
    };
    let (old, new) = (&old.manifest, &new.manifest);

    let mut blockers = Vec::new();
    if prefabs::is_draft(new) {
        // Brouillon : accepté sur un nœud de test, pour le mettre au point (ADR-0011).
        let mut conn = state.pool.acquire().await?;
        let on_test_node = crate::nodes::infra::NodeRepository::get(&mut conn, server.node_id)
            .await?
            .is_some_and(|node| node.trials);
        if !on_test_node {
            blockers.push(Blocker::NotDeployable);
        }
    }
    for (resource, current) in [
        ("memory_mb", server.memory_mb),
        ("cpu_percent", server.cpu_percent),
        ("disk_mb", server.disk_mb),
    ] {
        let unlimited = resource == "disk_mb" && current == 0;
        if let Some(minimum) = new.requires.resources.min(resource)
            && !unlimited
            && i64::from(current) < i64::from(minimum)
        {
            blockers.push(Blocker::ResourceBelowMinimum {
                resource,
                minimum,
                current,
            });
        }
    }
    let values = merged_values(state, server)?;
    let (renamed, removed, added) = match migrate(
        &from.unwrap_or_else(|| semver::Version::new(0, 0, 0)),
        new,
        &values,
    ) {
        Ok(migrated) => (migrated.renamed, migrated.removed, migrated.added),
        Err(errors) => {
            blockers.extend(setting_blockers(errors));
            (Vec::new(), Vec::new(), Vec::new())
        }
    };

    let (old_ports, new_ports) = (public_ports(old), public_ports(new));
    let ports_added: Vec<String> = new_ports.difference(&old_ports).cloned().collect();
    let ports_removed: Vec<String> = old_ports.difference(&new_ports).cloned().collect();
    let mut conn = state.pool.acquire().await?;
    if !ports_added.is_empty() {
        // Les ports retirés sont rendus avant d'allouer les nouveaux.
        let free = AllocationRepository::count_free(&mut conn, server.node_id).await?
            + i64::try_from(ports_removed.len()).unwrap_or_default();
        if free < i64::try_from(ports_added.len()).unwrap_or(i64::MAX) {
            blockers.push(Blocker::NoFreePort {
                needed: ports_added.len(),
                free,
            });
        }
    }
    for slot in old
        .content
        .keys()
        .filter(|slot| !new.content.contains_key(*slot))
    {
        let installed =
            crate::servers::infra::ContentRepository::of_slot(&mut conn, server.id, slot).await?;
        if !installed.is_empty() {
            blockers.push(Blocker::ContentSlot { slot: slot.clone() });
        }
    }
    drop(conn);

    let services: BTreeSet<&String> = old.services.keys().chain(new.services.keys()).collect();
    let images = services
        .into_iter()
        .filter_map(|service| {
            let from = old.services.get(service).map(|s| s.image.clone());
            let to = new.services.get(service).map(|s| s.image.clone());
            (from != to).then(|| ImageChange {
                service: service.clone(),
                from,
                to,
            })
        })
        .collect();
    let reinstall =
        serde_json::to_value(&old.install).ok() != serde_json::to_value(&new.install).ok();

    Ok(Preview {
        from: server.prefab_version.clone(),
        to: to.to_owned(),
        renamed,
        removed,
        added,
        ports_added,
        ports_removed,
        images,
        reinstall,
        blockers,
    })
}

/// Lance la mise à jour : vérifications immédiates, puis en tâche de fond la sauvegarde
/// (attendue) et le passage à la nouvelle version. Rend l'aperçu sur lequel elle s'appuie.
pub async fn start(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    to: &str,
    backup: bool,
) -> Result<Preview, ServersError> {
    let server = get(state, id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    if backup && server.backup_limit == 0 {
        return Err(ServersError::Conflict(ErrorCode::BackupsDisabled));
    }
    let preview = preview_of(state, &server, to).await?;
    if preview.blocked() {
        return Err(ServersError::Conflict(ErrorCode::UpgradeBlocked));
    }
    let (state, context, to) = (state.clone(), context.clone(), to.to_owned());
    tokio::spawn(async move {
        if let Err(reason) = run(&state, &context, id, &to, backup).await {
            tracing::warn!(server_id = %id, %reason, "server upgrade failed");
            let event = AuditEvent::failure(AuditAction::ServerUpgraded)
                .target("server", id)
                .details(json!({ "to": to, "reason": reason, "backup": backup }));
            if let Err(error) =
                crate::audit::application::record(&state.pool, &context, event).await
            {
                tracing::error!(%error, "failed to record the upgrade failure");
            }
        }
    });
    Ok(preview)
}

/// Sauvegarde attendue, puis mise à jour. Rend la cause d'un échec (code stable).
async fn run(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    to: &str,
    backup: bool,
) -> Result<(), String> {
    if backup {
        let created = backups::create(state, context, id, None)
            .await
            .map_err(|error| scheduler::reason_code(&error))?;
        scheduler::wait_for_backup(state, id, created.id).await?;
    }
    apply(state, context, id, to, backup)
        .await
        .map_err(|error| scheduler::reason_code(&error))
}

/// Passe le serveur à la version `to` et le redéploie.
async fn apply(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    to: &str,
    backup: bool,
) -> Result<(), ServersError> {
    // Aperçu, prefab et valeurs sont calculés hors transaction (une seule connexion à la fois) ;
    // le serveur est verrouillé ensuite, et refusé s'il a changé entre-temps.
    let server = get(state, id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    // L'état a pu changer pendant la sauvegarde : l'aperçu est recalculé.
    let preview = preview_of(state, &server, to).await?;
    if preview.blocked() {
        return Err(ServersError::Conflict(ErrorCode::UpgradeBlocked));
    }

    let package = prefabs::package(&state.pool, &server.prefab_id, to).await?;
    let from = parse(&server.prefab_version).unwrap_or_else(|| semver::Version::new(0, 0, 0));
    let mut values = migrate(&from, &package.manifest, &merged_values(state, &server)?)
        .map_err(super::settings_errors)?
        .values;
    for (key, length) in kuro_prefab::settings::secrets_to_generate(&package.manifest) {
        if let std::collections::btree_map::Entry::Vacant(entry) = values.entry(key) {
            entry.insert(Value::String(super::create::generate_secret(length)?));
        }
    }
    let (public, secrets) = split(&package.manifest, values);

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, id).await? {
        return Err(ServersError::NotFound);
    }
    let latest = ServerRepository::get(&mut tx, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    if latest.prefab_version != server.prefab_version || latest.state != server.state {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    for port in &preview.ports_removed {
        AllocationRepository::release(&mut tx, id, port).await?;
    }
    let mut assigned = Vec::new();
    for port in &preview.ports_added {
        let allocation = AllocationRepository::lock_first_free(&mut tx, server.node_id, &assigned)
            .await?
            .ok_or(ServersError::Conflict(ErrorCode::NoFreeAllocation))?;
        AllocationRepository::assign(&mut tx, allocation.0, id, port).await?;
        assigned.push(allocation.0);
    }
    ServerRepository::upgrade(
        &mut tx,
        id,
        to,
        &to_json(&public),
        seal_secrets(state, &secrets)?.as_deref(),
    )
    .await?;
    let running = server.state == ServerState::Running.as_str()
        || server.state == ServerState::Starting.as_str();
    if preview.reinstall && running {
        // L'installation laisse le serveur arrêté : il redémarre une fois installé.
        ServerRepository::set_start_when_installed(&mut tx, id).await?;
    }
    ServerRepository::set_state(&mut tx, id, ServerState::Deploying.as_str(), None).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerUpgraded)
            .target("server", id)
            .details(json!({
                "from": preview.from,
                "to": preview.to,
                "renamed": preview.renamed,
                "removed": preview.removed,
                "added": preview.added,
                "ports_added": preview.ports_added,
                "ports_removed": preview.ports_removed,
                "reinstall": preview.reinstall,
                "backup": backup,
            })),
    )
    .await?;

    // Étapes d'installation changées : elles accompagnent le déploiement, sur les volumes
    // existants (rien n'est effacé).
    if let Err(error) = deploy(state, id, preview.reinstall).await {
        record_failure(state, id, &error).await;
        return Err(error);
    }
    Ok(())
}

/// Serveurs du prefab `prefab_id` sur une version plus ancienne que `to`, avec leur aperçu.
pub async fn candidates(
    state: &ServersState,
    prefab_id: &str,
    to: &str,
) -> Result<Vec<(ServerRecord, Preview)>, ServersError> {
    let target = parse(to).ok_or(ServersError::Unprocessable(ErrorCode::UpgradeNotNewer))?;
    let mut conn = state.pool.acquire().await?;
    let servers = ServerRepository::of_prefab(&mut conn, prefab_id).await?;
    drop(conn);
    let mut out = Vec::new();
    for server in servers {
        let older = parse(&server.prefab_version).is_some_and(|v| v < target);
        if older {
            let preview = preview_of(state, &server, to).await?;
            out.push((server, preview));
        }
    }
    Ok(out)
}
