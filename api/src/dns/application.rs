//! Cas d'usage du DNS (ADR-0026) : configuration de la zone, labels des serveurs, adresses des
//! nœuds, et réconciliation des enregistrements voulus avec ceux que Kuro a créés.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::domain::{
    Desired, SrvPort, ZoneNames, node_records, server_records, suggest_label, valid_domain,
    valid_label, valid_sites_domain,
};
use super::infra::{DnsRepository, RecordRow, ZoneFields, ZoneRecord};
use super::provider::{DnsProvider, ProviderError, ZoneAccess};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::secret_box;
use crate::nodes::infra::AllocationRepository;
use crate::prefabs::application as prefabs;

/// Passe de réconciliation au plus tard toutes les 60 s ; un changement fait par l'API la
/// réveille aussitôt.
const RECONCILE_EVERY: Duration = Duration::from_secs(60);
const DEFAULT_TTL: i32 = 300;
pub const PROVIDERS: [&str; 1] = ["cloudflare"];

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("dns is not configured")]
    NotConfigured,
    #[error("invalid domain")]
    InvalidDomain,
    #[error("invalid label")]
    InvalidLabel,
    #[error("invalid sites domain")]
    InvalidSitesDomain,
    #[error("label already taken")]
    LabelTaken,
    #[error("invalid ip address")]
    InvalidAddress,
    #[error("unknown dns provider")]
    UnknownProvider,
    #[error("a token is required")]
    TokenRequired,
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl From<secret_box::SecretBoxError> for DnsError {
    fn from(error: secret_box::SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct DnsState {
    pub pool: PgPool,
    /// Clé de chiffrement du jeton (celle des secrets des serveurs).
    pub key: [u8; 32],
    pub provider: Arc<dyn DnsProvider>,
    wake: Arc<Notify>,
    /// Une seule réconciliation à la fois.
    running: Arc<Mutex<()>>,
}

impl DnsState {
    pub fn new(pool: PgPool, key: [u8; 32], provider: Arc<dyn DnsProvider>) -> Self {
        Self {
            pool,
            key,
            provider,
            wake: Arc::default(),
            running: Arc::default(),
        }
    }

    /// Demande une réconciliation au plus tôt.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    fn access(&self, zone: &ZoneRecord) -> Result<ZoneAccess, DnsError> {
        let token = secret_box::open(&self.key, &zone.token)?;
        Ok(ZoneAccess {
            zone: zone.provider_zone.clone(),
            token: String::from_utf8(token).map_err(|e| DnsError::Internal(e.to_string()))?,
            ttl: u32::try_from(zone.ttl).unwrap_or(300),
        })
    }
}

/// Configuration saisie par l'admin ; `token` absent : celui déjà enregistré est gardé.
#[derive(Debug, Clone)]
pub struct ZoneInput {
    pub provider: String,
    pub base_domain: String,
    pub nodes_label: String,
    /// Domaine des sites web (ADR-0038) ; vide ou absent : aucun site.
    pub sites_domain: Option<String>,
    pub token: Option<String>,
    pub ttl: Option<i32>,
    pub enabled: bool,
}

pub async fn zone(state: &DnsState) -> Result<Option<ZoneRecord>, DnsError> {
    let mut conn = state.pool.acquire().await?;
    Ok(DnsRepository::zone(&mut conn).await?)
}

/// Enregistre la zone ; le fournisseur doit reconnaître le jeton et la zone qui contient le
/// domaine de base.
pub async fn configure(
    state: &DnsState,
    context: &AuditContext,
    input: ZoneInput,
) -> Result<ZoneRecord, DnsError> {
    let base = input
        .base_domain
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let nodes_label = input.nodes_label.trim().to_ascii_lowercase();
    if !valid_domain(&base) {
        return Err(DnsError::InvalidDomain);
    }
    if !valid_label(&nodes_label) {
        return Err(DnsError::InvalidLabel);
    }
    let sites = input
        .sites_domain
        .as_deref()
        .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    if let Some(sites) = &sites
        && !valid_sites_domain(sites, &base, &nodes_label)
    {
        return Err(DnsError::InvalidSitesDomain);
    }
    if !PROVIDERS.contains(&input.provider.as_str()) {
        return Err(DnsError::UnknownProvider);
    }
    let ttl = input.ttl.unwrap_or(DEFAULT_TTL).clamp(60, 86_400);
    let current = zone(state).await?;
    let token = match (
        input
            .token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty()),
        &current,
    ) {
        (Some(token), _) => token.to_owned(),
        (None, Some(zone)) => state.access(zone)?.token,
        (None, None) => return Err(DnsError::TokenRequired),
    };
    let provider_zone = state.provider.find_zone(&token, &base).await?;
    // Le domaine des sites vit dans la même zone du fournisseur que celui des jeux (un seul
    // jeton) ; un domaine séparé viendra comme seconde zone (ADR-0038).
    if let Some(sites) = &sites {
        match state.provider.find_zone(&token, sites).await {
            Ok(zone) if zone == provider_zone => {}
            Ok(_) | Err(ProviderError::ZoneNotFound) => return Err(DnsError::InvalidSitesDomain),
            Err(error) => return Err(error.into()),
        }
    }
    let sealed = secret_box::seal(&state.key, token.as_bytes())?;
    let fields = ZoneFields {
        provider: &input.provider,
        base_domain: &base,
        nodes_label: &nodes_label,
        sites_domain: sites.as_deref(),
        provider_zone: &provider_zone,
        token: &sealed,
        ttl,
        enabled: input.enabled,
    };
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let id = match &current {
        Some(zone) => {
            DnsRepository::update_zone(&mut tx, zone.id, &fields).await?;
            zone.id
        }
        None => {
            let id = Uuid::now_v7();
            DnsRepository::insert_zone(&mut tx, id, &fields).await?;
            id
        }
    };
    tx.commit(
        AuditEvent::success(AuditAction::DnsZoneUpdated)
            .target("dns_zone", id)
            .details(json!({
                "provider": input.provider,
                "base_domain": base,
                "nodes_label": nodes_label,
                "sites_domain": sites,
                "ttl": ttl,
                "enabled": input.enabled,
                "token_changed": input.token.is_some(),
            })),
    )
    .await?;
    state.wake();
    zone(state).await?.ok_or(DnsError::NotConfigured)
}

/// Vérifie que le jeton enregistré ouvre toujours la zone.
pub async fn test(state: &DnsState) -> Result<usize, DnsError> {
    let zone = zone(state).await?.ok_or(DnsError::NotConfigured)?;
    let access = state.access(&zone)?;
    Ok(state.provider.list(&access).await?.len())
}

/// Supprime la zone : les enregistrements créés par Kuro sont d'abord effacés chez le
/// fournisseur (au mieux), puis la configuration.
pub async fn remove(state: &DnsState, context: &AuditContext) -> Result<(), DnsError> {
    let _guard = state.running.lock().await;
    let zone = zone(state).await?.ok_or(DnsError::NotConfigured)?;
    let access = state.access(&zone)?;
    let mut conn = state.pool.acquire().await?;
    for row in DnsRepository::records(&mut conn, zone.id).await? {
        if let Some(id) = &row.provider_id
            && let Err(error) = state.provider.delete(&access, id).await
        {
            tracing::warn!(%error, name = %row.name, "dns record left at the provider");
        }
    }
    drop(conn);
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    DnsRepository::delete_zone(&mut tx, zone.id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::DnsZoneRemoved)
            .target("dns_zone", zone.id)
            .details(json!({ "base_domain": zone.base_domain })),
    )
    .await?;
    Ok(())
}

/// Change le label d'un serveur (`None` : plus de nom).
pub async fn set_label(
    state: &DnsState,
    context: &AuditContext,
    server: Uuid,
    label: Option<&str>,
) -> Result<(), DnsError> {
    if label.is_some_and(|l| !valid_label(l)) {
        return Err(DnsError::InvalidLabel);
    }
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM servers WHERE id = $1) AS "exists!""#,
        server
    )
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(DnsError::NotFound);
    }
    if !DnsRepository::set_label(&mut tx, server, label).await? {
        return Err(DnsError::LabelTaken);
    }
    tx.commit(
        AuditEvent::success(AuditAction::ServerDnsLabelChanged)
            .target("server", server)
            .details(json!({ "label": label })),
    )
    .await?;
    state.wake();
    Ok(())
}

/// Adresses publiques d'un nœud (vides : retirées).
pub async fn set_node_addresses(
    state: &DnsState,
    context: &AuditContext,
    node: Uuid,
    ipv4: Option<&str>,
    ipv6: Option<&str>,
) -> Result<(), DnsError> {
    let ipv4 = ipv4.map(str::trim).filter(|v| !v.is_empty());
    let ipv6 = ipv6.map(str::trim).filter(|v| !v.is_empty());
    if ipv4.is_some_and(|v| v.parse::<std::net::Ipv4Addr>().is_err())
        || ipv6.is_some_and(|v| v.parse::<std::net::Ipv6Addr>().is_err())
    {
        return Err(DnsError::InvalidAddress);
    }
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM nodes WHERE id = $1) AS "exists!""#,
        node
    )
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Err(DnsError::NotFound);
    }
    DnsRepository::set_node_addresses(&mut tx, node, ipv4, ipv6).await?;
    tx.commit(
        AuditEvent::success(AuditAction::NodeAddressUpdated)
            .target("node", node)
            .details(json!({ "ipv4": ipv4, "ipv6": ipv6 })),
    )
    .await?;
    state.wake();
    Ok(())
}

/// Bilan d'une réconciliation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub created: usize,
    pub updated: usize,
    pub deleted: usize,
    pub failed: usize,
}

/// Enregistrements voulus : propriétaire (`node` ou `server`, identifiant) et enregistrement.
/// Donne un label aux serveurs qui n'en ont pas encore.
async fn desired(
    state: &DnsState,
    names: &ZoneNames,
) -> Result<Vec<(&'static str, Uuid, Desired)>, DnsError> {
    let mut conn = state.pool.acquire().await?;
    let mut out = Vec::new();
    for node in DnsRepository::nodes(&mut conn).await? {
        let ipv4 = node.public_ipv4.as_deref().and_then(|v| v.parse().ok());
        let ipv6 = node.public_ipv6.as_deref().and_then(|v| v.parse().ok());
        for record in node_records(names, &node.name, ipv4, ipv6) {
            out.push(("node", node.id, record));
        }
    }
    let mut taken: HashSet<String> = DnsRepository::labels(&mut conn)
        .await?
        .into_iter()
        .collect();
    for server in DnsRepository::servers(&mut conn).await? {
        let label = match server.dns_label.clone() {
            Some(label) => label,
            None => {
                let base = suggest_label(&server.name);
                let label = (1..)
                    .map(|n| {
                        if n == 1 {
                            base.clone()
                        } else {
                            format!("{base}-{n}")
                        }
                    })
                    .find(|candidate| !taken.contains(candidate))
                    .unwrap_or(base);
                if !DnsRepository::set_label(&mut conn, server.id, Some(&label)).await? {
                    continue;
                }
                taken.insert(label.clone());
                label
            }
        };
        let ports: HashMap<String, u16> = AllocationRepository::of_server(&mut conn, server.id)
            .await?
            .into_iter()
            .map(|(name, _, port)| (name, u16::try_from(port).unwrap_or_default()))
            .collect();
        let srv: Vec<SrvPort> =
            match prefabs::package(&state.pool, &server.prefab_id, &server.prefab_version).await {
                Ok(package) => package
                    .manifest
                    .services
                    .values()
                    .flat_map(|service| service.ports.iter())
                    .filter_map(|(name, decl)| {
                        Some(SrvPort {
                            service: decl.srv.clone()?,
                            port: *ports.get(name)?,
                        })
                    })
                    .collect(),
                Err(error) => {
                    tracing::warn!(%error, server = %server.id, "prefab unreadable, no srv record");
                    Vec::new()
                }
            };
        for record in server_records(
            names,
            &label,
            &server.node_name,
            &srv,
            server.web_id.as_deref(),
        ) {
            out.push(("server", server.id, record));
        }
    }
    Ok(out)
}

fn audit_record(
    action: AuditAction,
    row_name: &str,
    kind: &str,
    content: Option<&str>,
) -> AuditEvent {
    AuditEvent::success(action)
        .target("dns_record", row_name)
        .details(json!({ "kind": kind, "content": content }))
}

/// Aligne les enregistrements créés par Kuro sur ceux voulus : crée, corrige, recrée ceux
/// effacés à la main chez le fournisseur, supprime ceux qui ne servent plus. Ne touche jamais
/// un enregistrement que Kuro n'a pas créé ; un échec est noté sur l'enregistrement et
/// réessayé à la passe suivante.
pub async fn reconcile(state: &DnsState) -> Result<Report, DnsError> {
    let _guard = state.running.lock().await;
    // Une seule instance de l'API réconcilie à la fois (ADR-0037) : verrou consultatif de
    // session, tenu par cette connexion le temps de la passe.
    let mut lock = state.pool.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(RECONCILE_LOCK)
        .fetch_one(&mut *lock)
        .await?;
    if !locked {
        return Ok(Report::default());
    }
    let result = reconcile_locked(state).await;
    if let Err(error) = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(RECONCILE_LOCK)
        .execute(&mut *lock)
        .await
    {
        // Le verrou tombera avec la connexion ; d'ici là, les autres instances passent leur tour.
        tracing::warn!(%error, "failed to release the dns reconciliation lock");
    }
    result
}

/// Clé du verrou consultatif de la réconciliation.
const RECONCILE_LOCK: i64 = 0x6b75_726f_5f64_6e73; // "kuro_dns"

async fn reconcile_locked(state: &DnsState) -> Result<Report, DnsError> {
    let Some(zone) = zone(state).await? else {
        return Ok(Report::default());
    };
    if !zone.enabled {
        return Ok(Report::default());
    }
    let access = state.access(&zone)?;
    let names = ZoneNames {
        base: zone.base_domain.clone(),
        nodes_label: zone.nodes_label.clone(),
        sites: zone.sites_domain.clone(),
    };
    let wanted = desired(state, &names).await?;
    let remote_ids: HashSet<String> = state
        .provider
        .list(&access)
        .await?
        .into_iter()
        .map(|remote| remote.id)
        .collect();

    let context = AuditContext::system();
    let mut conn = state.pool.acquire().await?;
    let rows = DnsRepository::records(&mut conn, zone.id).await?;
    let mut by_key: HashMap<(String, String), RecordRow> = rows
        .into_iter()
        .map(|row| ((row.kind.clone(), row.name.clone()), row))
        .collect();
    let mut report = Report::default();

    for (owner_kind, owner_id, record) in &wanted {
        let key = (record.kind.as_str().to_owned(), record.name.clone());
        let row = match by_key.remove(&key) {
            Some(row) => row,
            None => {
                let id = Uuid::now_v7();
                DnsRepository::insert_record(
                    &mut conn,
                    id,
                    zone.id,
                    owner_kind,
                    *owner_id,
                    record.kind.as_str(),
                    &record.name,
                    &record.content,
                )
                .await?;
                RecordRow {
                    id,
                    owner_kind: (*owner_kind).to_owned(),
                    owner_id: *owner_id,
                    kind: record.kind.as_str().to_owned(),
                    name: record.name.clone(),
                    content: record.content.clone(),
                    provider_id: None,
                    synced_at: None,
                    error: None,
                }
            }
        };
        let published = row
            .provider_id
            .as_ref()
            .filter(|id| remote_ids.contains(*id));
        let outcome = match published {
            None => state
                .provider
                .create(&access, record)
                .await
                .map(|id| (id, AuditAction::DnsRecordCreated)),
            Some(id) if row.content != record.content || row.error.is_some() => state
                .provider
                .update(&access, id, record)
                .await
                .map(|()| (id.clone(), AuditAction::DnsRecordUpdated)),
            Some(_) => continue,
        };
        match outcome {
            Ok((provider_id, action)) => {
                DnsRepository::mark_synced(
                    &mut conn,
                    row.id,
                    &provider_id,
                    &record.content,
                    *owner_id,
                )
                .await?;
                if action == AuditAction::DnsRecordCreated {
                    report.created += 1;
                } else {
                    report.updated += 1;
                }
                crate::audit::application::record(
                    &state.pool,
                    &context,
                    audit_record(
                        action,
                        &record.name,
                        record.kind.as_str(),
                        Some(&record.content),
                    ),
                )
                .await
                .map_err(|e| DnsError::Internal(e.to_string()))?;
            }
            Err(error) => {
                report.failed += 1;
                tracing::warn!(%error, name = %record.name, "dns record not published");
                DnsRepository::mark_error(&mut conn, row.id, &error.to_string()).await?;
            }
        }
    }

    // Ce qui reste n'est plus voulu : nœud ou serveur supprimé, label ou zone changés.
    for row in by_key.into_values() {
        if let Some(id) = &row.provider_id
            && let Err(error) = state.provider.delete(&access, id).await
        {
            report.failed += 1;
            tracing::warn!(%error, name = %row.name, "dns record not deleted");
            DnsRepository::mark_error(&mut conn, row.id, &error.to_string()).await?;
            continue;
        }
        DnsRepository::delete_record(&mut conn, row.id).await?;
        report.deleted += 1;
        crate::audit::application::record(
            &state.pool,
            &context,
            audit_record(AuditAction::DnsRecordDeleted, &row.name, &row.kind, None),
        )
        .await
        .map_err(|e| DnsError::Internal(e.to_string()))?;
    }
    Ok(report)
}

/// Boucle de l'API : une réconciliation par minute, ou dès qu'un changement la réveille.
pub async fn run_forever(state: DnsState, shutdown: CancellationToken) {
    loop {
        match reconcile(&state).await {
            Ok(report) if report != Report::default() => {
                tracing::info!(?report, "dns records reconciled");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "dns reconciliation failed"),
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = state.wake.notified() => {
                // Regroupe les changements faits coup sur coup.
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            () = tokio::time::sleep(RECONCILE_EVERY) => {}
        }
    }
}

/// Noms publics de la zone, pour afficher l'adresse d'un serveur.
pub fn names(zone: &ZoneRecord) -> ZoneNames {
    ZoneNames {
        base: zone.base_domain.clone(),
        nodes_label: zone.nodes_label.clone(),
        sites: zone.sites_domain.clone(),
    }
}
