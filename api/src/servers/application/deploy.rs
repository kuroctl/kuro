//! Résolution du prefab et envoi de `server.deploy` au nœud du serveur.

use std::collections::BTreeMap;

use kuro_prefab::{Deployment, Package, resolve, resolve_install, with_steam_login};
use kuro_protocol::ApiMessage;
use kuro_protocol::api::install::InstallSpec;
use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use uuid::Uuid;

use super::{ServersError, ServersState, merged_values};
use crate::error_code::ErrorCode;
use crate::nodes::domain::NodeId;
use crate::nodes::infra::AllocationRepository;
use crate::nodes::infra::NodeRepository;
use crate::prefabs::application as prefabs;
use crate::servers::domain::ServerState;
use crate::servers::infra::{ContentRepository, ServerRecord, ServerRepository};

/// Spécification de déploiement et étapes d'installation du serveur, avec son nœud.
pub(super) struct Resolved {
    pub node: NodeId,
    pub deploy: ServerDeploy,
    pub install: Option<InstallSpec>,
    pub package: Package,
}

/// Résout le prefab du serveur avec ses réglages, ses ressources et ses ports.
pub(super) async fn resolved(state: &ServersState, id: Uuid) -> Result<Resolved, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, id)
        .await?
        .ok_or(ServersError::NotFound)?;
    drop(conn);
    resolve_server(state, &server).await
}

/// Ce qui décide d'un déploiement, lu en base : possédé, pour construire un [`Deployment`]
/// autant de fois que nécessaire (déploiement, installation, action).
pub(super) struct Inputs {
    server_id: Uuid,
    server_name: String,
    values: kuro_prefab::settings::Values,
    memory_mb: u32,
    cpu_percent: u32,
    allocations: BTreeMap<String, (String, u16)>,
    web_url: Option<String>,
    content: BTreeMap<String, Vec<String>>,
    content_values: BTreeMap<String, Vec<String>>,
    packages: BTreeMap<String, Vec<serde_json::Value>>,
}

impl Inputs {
    pub(super) fn deployment(&self) -> Deployment<'_> {
        Deployment {
            server_id: self.server_id,
            server_name: &self.server_name,
            settings: &self.values,
            memory_mb: self.memory_mb,
            cpu_percent: self.cpu_percent,
            allocations: &self.allocations,
            web_url: self.web_url.clone(),
            content: self.content.clone(),
            content_values: self.content_values.clone(),
            packages: self.packages.clone(),
        }
    }
}

/// Prefab du serveur et entrées de son déploiement.
pub(super) async fn inputs(
    state: &ServersState,
    server: &ServerRecord,
) -> Result<(Package, Inputs), ServersError> {
    let id = server.id;
    let mut conn = state.pool.acquire().await?;
    let allocations: BTreeMap<String, (String, u16)> =
        AllocationRepository::of_server(&mut conn, id)
            .await?
            .into_iter()
            .map(|(port, ip, number)| (port, (ip, u16::try_from(number).unwrap_or_default())))
            .collect();
    // Contenu en mode `reference` : le jeu le télécharge lui-même, d'après ces identifiants.
    let mut content: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (slot, project_id) in ContentRepository::reference_ids(&mut conn, id).await? {
        content.entry(slot).or_default().push(project_id);
    }
    // Valeurs saisies (ADR-0030) : `<emplacement>.<champ>` → valeurs, contenu après contenu.
    let mut content_values: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (slot, values) in ContentRepository::active_field_values(&mut conn, id).await? {
        for (field, list) in values.as_object().into_iter().flatten() {
            let entry = content_values.entry(format!("{slot}.{field}")).or_default();
            entry.extend(
                list.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned)),
            );
        }
    }
    // Paquets extraits actifs (ADR-0030) : les entrées des fichiers d'inscription.
    let mut packages: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for (slot, description) in ContentRepository::active_packages(&mut conn, id).await? {
        packages.entry(slot).or_default().push(description);
    }
    drop(conn);

    let package = prefabs::package(&state.pool, &server.prefab_id, &server.prefab_version).await?;
    let values = merged_values(state, server)?;
    let inputs = Inputs {
        server_id: id,
        server_name: server.name.clone(),
        values,
        memory_mb: u32::try_from(server.memory_mb).unwrap_or_default(),
        cpu_percent: u32::try_from(server.cpu_percent).unwrap_or_default(),
        allocations,
        web_url: super::web::web_url(&state.pool, server).await?,
        content,
        content_values,
        packages,
    };
    Ok((package, inputs))
}

/// Comme [`resolved`], pour un enregistrement donné (ressources pas encore enregistrées).
pub(super) async fn resolve_server(
    state: &ServersState,
    server: &ServerRecord,
) -> Result<Resolved, ServersError> {
    let (package, inputs) = inputs(state, server).await?;
    let deployment = inputs.deployment();
    let internal = |e: kuro_prefab::ResolveError| ServersError::Internal(e.to_string());
    let mut deploy = resolve(&package, &deployment).map_err(internal)?;
    deploy.disk_bytes = disk_bytes(server.disk_mb);
    deploy.suspended = server.suspended_at.is_some();
    let install = resolve_install(&package, &deployment).map_err(internal)?;
    Ok(Resolved {
        node: NodeId::from(server.node_id),
        deploy,
        install,
        package,
    })
}

/// Étapes d'installation prêtes à partir vers le nœud : celles qui l'exigent reçoivent le compte
/// Steam de l'installation (ADR-0033), déchiffré au dernier moment.
pub(super) async fn with_steam_account(
    state: &ServersState,
    package: &Package,
    install: Option<InstallSpec>,
) -> Result<Option<InstallSpec>, ServersError> {
    let Some(mut install) = install else {
        return Ok(None);
    };
    if package.manifest.needs_steam_account() {
        let account = super::steam_account::login(state).await?;
        with_steam_login(package, &mut install, account.as_ref())
            .map_err(|_| ServersError::Unprocessable(ErrorCode::SteamAccountRequired))?;
    }
    Ok(Some(install))
}

/// Emplacements de contenu du serveur, gabarits rendus avec ses réglages (ADR-0020).
pub(super) async fn content_plans(
    state: &ServersState,
    server: &ServerRecord,
) -> Result<Vec<kuro_prefab::ContentPlan>, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let allocations: BTreeMap<String, (String, u16)> =
        AllocationRepository::of_server(&mut conn, server.id)
            .await?
            .into_iter()
            .map(|(port, ip, number)| (port, (ip, u16::try_from(number).unwrap_or_default())))
            .collect();
    drop(conn);
    let package = prefabs::package(&state.pool, &server.prefab_id, &server.prefab_version).await?;
    let values = merged_values(state, server)?;
    let deployment = Deployment {
        server_id: server.id,
        server_name: &server.name,
        settings: &values,
        memory_mb: u32::try_from(server.memory_mb).unwrap_or_default(),
        cpu_percent: u32::try_from(server.cpu_percent).unwrap_or_default(),
        allocations: &allocations,
        web_url: super::web::web_url(&state.pool, server).await?,
        content: BTreeMap::new(),
        content_values: BTreeMap::new(),
        packages: BTreeMap::new(),
    };
    kuro_prefab::resolve_content(&package, &deployment)
        .map_err(|e| ServersError::Internal(e.to_string()))
}

/// Quota disque en octets ; `0` : illimité.
pub(super) fn disk_bytes(disk_mb: i32) -> u64 {
    u64::try_from(disk_mb).unwrap_or(0) * 1024 * 1024
}

/// Déploie (ou redéploie) le serveur sur son nœud. `first` : les étapes d'installation du
/// prefab accompagnent le déploiement, à la création (ADR-0017) ou lors d'une mise à jour qui
/// les change (ADR-0024, sur les volumes existants) ; un redéploiement après modification des
/// réglages ne réinstalle jamais.
pub(super) async fn deploy(
    state: &ServersState,
    id: Uuid,
    first: bool,
) -> Result<(), ServersError> {
    deploy_with(state, id, first, false).await
}

/// Comme [`deploy`], sans redémarrer un serveur en marche : le nœud garde la spécification et
/// l'applique au prochain démarrage (changement de contenu).
pub(super) async fn deploy_deferred(state: &ServersState, id: Uuid) -> Result<(), ServersError> {
    deploy_with(state, id, false, true).await
}

async fn deploy_with(
    state: &ServersState,
    id: Uuid,
    first: bool,
    defer: bool,
) -> Result<(), ServersError> {
    let Resolved {
        node,
        mut deploy,
        install,
        package,
    } = resolved(state, id).await?;
    if first {
        deploy.install = with_steam_account(state, &package, install).await?;
    }
    deploy.defer = defer;
    // Daemon trop ancien pour ce que le prefab demande : refusé avant l'envoi (ADR-0039).
    {
        let mut conn = state.pool.acquire().await?;
        if let Some(record) = NodeRepository::get(&mut conn, node.as_uuid()).await? {
            super::compat::check(&record.name, record.daemon_version.as_deref(), &deploy)?;
        }
    }
    let result = state
        .commands
        .send(node, ApiMessage::ServerDeploy(deploy))
        .await?;
    accepted(result)
}

/// Comme [`deploy`], mais un échec passe le serveur en `failed` avec sa raison.
pub(super) async fn deploy_recording_failure(state: &ServersState, id: Uuid, first: bool) {
    if let Err(error) = deploy(state, id, first).await {
        tracing::warn!(server_id = %id, %error, "server deployment failed");
        record_failure(state, id, &error).await;
    }
}

pub(super) async fn record_failure(state: &ServersState, id: Uuid, error: &ServersError) {
    let reason = match error {
        ServersError::Conflict(code) | ServersError::Unprocessable(code) => {
            code.as_str().to_owned()
        }
        ServersError::Daemon { code, .. } => code.as_str().to_owned(),
        _ => ErrorCode::InternalError.as_str().to_owned(),
    };
    let outcome = async {
        let mut conn = state.pool.acquire().await?;
        ServerRepository::set_state(&mut conn, id, ServerState::Failed.as_str(), Some(&reason))
            .await
    };
    if let Err(db) = outcome.await {
        tracing::error!(server_id = %id, %db, "failed to record the deployment failure");
    }
}

/// Réponse du daemon : acceptée, ou refus traduit en erreur stable.
pub(super) fn accepted(result: CommandResult) -> Result<(), ServersError> {
    match result {
        CommandResult::Ok => Ok(()),
        CommandResult::Error {
            code: CommandErrorCode::Busy,
            ..
        } => Err(ServersError::Conflict(ErrorCode::ServerBusy)),
        CommandResult::Error {
            code: CommandErrorCode::DiskQuotaExceeded,
            ..
        } => Err(ServersError::Conflict(ErrorCode::DiskQuotaExceeded)),
        CommandResult::Error {
            code: CommandErrorCode::Suspended,
            ..
        } => Err(ServersError::Conflict(ErrorCode::ServerSuspended)),
        CommandResult::Error { code, message } => Err(ServersError::Daemon {
            code: ErrorCode::DaemonError,
            message: format!("{code:?}: {message}"),
        }),
    }
}
