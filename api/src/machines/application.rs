//! Cas d'usage des nœuds à la demande (ADR-0065) : réglages, location, retour, sommeil, réveil,
//! création d'un serveur en attente du nœud, passage de ménage. Les fournisseurs ne font que
//! traduire le trait `Provider`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

use super::domain::{
    ANNOUNCED_STEPS, Action, DISK_RESERVE_GB, ENROLL_TTL, LABEL, MachineState, NO_NEWS_ERROR,
    Resources, RowState, SCOPE_LABEL, cheapest_covering, cloud_init, is_location, machine_name,
    machine_step, memory_reserve_mb, short_error, silent_step,
};
use super::provider::{
    Listed, Location, MachineSpec, Offer, Provider, ProviderError, SnapshotState,
};
use super::providers::{KNOWN, hetzner::Hetzner};
use crate::audit::domain::{Actor, ActorKind, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::secret_box;
use crate::enrollment::application::{AddNodeError, add_node};
use crate::error_code::{ErrorCode, FieldError};
use crate::extension::{HookError, MountedExtension};
use crate::nodes::application::{NodesError, PlacementInput, allocations, lifecycle};
use crate::nodes::domain::{NodeLinks, NodeStatus};
use crate::nodes::infra::NodeRepository;
use crate::servers::application::{NewServer, ServersState};
use crate::tasks::{Task, TaskDef, TaskError};

/// Passage de ménage (tâche `machines.sweep`).
pub const SWEEP: TaskDef = TaskDef {
    name: "sweep",
    period: Duration::from_secs(60),
    dry_run_by_default: true,
};
const HOOK_TIMEOUT: Duration = Duration::from_secs(10);
/// Document cloud-init d'une VM recréée depuis un snapshot : le nœud y est déjà installé.
const NOTHING_TO_INIT: &str = "#cloud-config\n";

// Événements pour les extensions.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineEventKind {
    /// VM créée chez le fournisseur.
    Ready,
    /// Nœud en ligne pour la première fois.
    NodeOnline,
    /// Nœud jamais en ligne dans le délai (machine réclamée : à l'appelant de décider).
    Stalled,
    /// VM disparue chez le fournisseur.
    Lost,
    Asleep,
    Awake,
    Released,
}

#[derive(Debug, Clone)]
pub struct MachineEvent {
    pub machine_id: Uuid,
    pub node_id: Option<Uuid>,
    pub claim: Option<String>,
    pub kind: MachineEventKind,
}

/// Événements des machines (ADR-0065 § 8) : reçus après la validation de leur transaction, dans
/// une tâche à part, au plus une fois ; l'extension se resynchronise en relisant `machines`.
pub trait MachineHooks: Send + Sync {
    fn on_machine_event<'a>(
        &'a self,
        event: &'a MachineEvent,
    ) -> BoxFuture<'a, Result<(), HookError>>;
}

#[derive(Clone, Default)]
pub struct MachineEvents(Arc<Vec<(&'static str, Arc<dyn MachineHooks>)>>);

impl MachineEvents {
    pub fn of(extensions: &[MountedExtension]) -> Self {
        Self(Arc::new(
            extensions
                .iter()
                .filter_map(|e| e.machine_hooks.clone().map(|hooks| (e.name, hooks)))
                .collect(),
        ))
    }

    pub fn emit(&self, event: MachineEvent) {
        if self.0.is_empty() {
            return;
        }
        let listeners = self.0.clone();
        tokio::spawn(async move {
            for (extension, hooks) in listeners.iter() {
                let outcome =
                    tokio::time::timeout(HOOK_TIMEOUT, hooks.on_machine_event(&event)).await;
                let error = match outcome {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => error.to_string(),
                    Err(_) => "timed out".to_owned(),
                };
                tracing::warn!(
                    extension,
                    machine_id = %event.machine_id,
                    kind = ?event.kind,
                    %error,
                    "extension failed to handle a machine event"
                );
            }
        });
    }
}

impl std::fmt::Debug for MachineEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MachineEvents")
            .field(&self.0.iter().map(|(name, _)| *name).collect::<Vec<_>>())
            .finish()
    }
}

// État et erreurs.

#[derive(Clone)]
pub struct MachinesState {
    pub pool: PgPool,
    /// Clé de chiffrement des identifiants et des créations en attente (celle des secrets des
    /// serveurs).
    pub key: [u8; 32],
    /// Empreinte de la CA, passée à l'installeur du nœud.
    pub ca_fingerprint: String,
    pub links: Arc<dyn NodeLinks>,
    /// Création d'un serveur en attente du nœud.
    pub servers: ServersState,
    pub events: MachineEvents,
    /// Fournisseur imposé (tests), à la place de celui des réglages.
    pub fixed: Option<Arc<dyn Provider>>,
}

impl std::fmt::Debug for MachinesState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachinesState")
            .field("events", &self.events)
            .field("fixed", &self.fixed)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MachinesError {
    #[error("not found")]
    NotFound,
    #[error("invalid {}: {}", .0.field, .0.code.as_str())]
    Invalid(FieldError),
    #[error("unprocessable: {}", .0.as_str())]
    Unprocessable(ErrorCode),
    #[error("conflict: {}", .0.as_str())]
    Conflict(ErrorCode),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Nodes(#[from] NodesError),
    #[error(transparent)]
    Server(crate::servers::application::ServersError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("{0}")]
    Internal(String),
}

impl From<secret_box::SecretBoxError> for MachinesError {
    fn from(error: secret_box::SecretBoxError) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<AddNodeError> for MachinesError {
    fn from(error: AddNodeError) -> Self {
        match error {
            AddNodeError::Placement(error) => Self::Nodes(error),
            AddNodeError::Database(error) => Self::Database(error),
            other => Self::Internal(other.to_string()),
        }
    }
}

// Réglages.

struct SettingsRow {
    instance_tag: String,
    api_url: Option<String>,
    port_range: String,
}

async fn load(pool: &PgPool) -> Result<SettingsRow, sqlx::Error> {
    sqlx::query_as!(
        SettingsRow,
        "SELECT instance_tag, api_url, port_range FROM machine_settings"
    )
    .fetch_one(pool)
    .await
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProviderStatus {
    /// `hetzner`.
    pub name: String,
    /// Des identifiants sont enregistrés (jamais renvoyés).
    pub configured: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub updated_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RegionHosting {
    pub region_id: Uuid,
    pub slug: String,
    pub name: String,
    /// Fournisseur de la région ; absent : la région ne loue rien.
    pub provider: Option<String>,
    /// Emplacement chez le fournisseur (`fsn1`).
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MachinesSettings {
    /// Adresse de l'API vue par les machines (installeur du nœud).
    pub api_url: Option<String>,
    /// Ports des serveurs, ouverts et alloués sur chaque machine.
    pub port_range: String,
    pub providers: Vec<ProviderStatus>,
    pub regions: Vec<RegionHosting>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct RegionHostingInput {
    pub region_id: Uuid,
    pub provider: String,
    pub location: String,
}

#[derive(Debug, Clone)]
pub struct SettingsInput {
    pub api_url: Option<String>,
    pub port_range: String,
    /// Remplace toutes les régions louées.
    pub regions: Vec<RegionHostingInput>,
}

pub async fn settings(pool: &PgPool) -> Result<MachinesSettings, MachinesError> {
    let row = load(pool).await?;
    let saved = sqlx::query!("SELECT provider, updated_at FROM machine_providers")
        .fetch_all(pool)
        .await?;
    let providers = KNOWN
        .iter()
        .map(|name| {
            let saved = saved.iter().find(|p| p.provider == *name);
            ProviderStatus {
                name: (*name).to_owned(),
                configured: saved.is_some(),
                updated_at: saved.map(|p| p.updated_at),
            }
        })
        .collect();
    let regions = sqlx::query_as!(
        RegionHosting,
        r#"SELECT r.id AS region_id, r.slug, r.name,
                  h.provider AS "provider?", h.location AS "location?"
             FROM regions r LEFT JOIN machine_regions h ON h.region_id = r.id
            ORDER BY r.name"#
    )
    .fetch_all(pool)
    .await?;
    Ok(MachinesSettings {
        api_url: row.api_url,
        port_range: row.port_range,
        providers,
        regions,
    })
}

fn is_api_url(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://"))
        && value.len() <= 500
        && !value.chars().any(char::is_whitespace)
        && value
            .split_once("://")
            .is_some_and(|(_, rest)| !rest.is_empty())
}

pub async fn update_settings(
    pool: &PgPool,
    context: &AuditContext,
    input: SettingsInput,
) -> Result<MachinesSettings, MachinesError> {
    let api_url = input
        .api_url
        .map(|url| url.trim().trim_end_matches('/').to_owned())
        .filter(|url| !url.is_empty());
    if api_url.as_deref().is_some_and(|url| !is_api_url(url)) {
        return Err(MachinesError::Invalid(FieldError::new(
            "api_url",
            ErrorCode::InvalidApiUrl,
        )));
    }
    let port_range = input.port_range.trim().to_owned();
    if allocations::parse_ports(&port_range).is_none() {
        return Err(MachinesError::Invalid(FieldError::new(
            "port_range",
            ErrorCode::InvalidPortRange,
        )));
    }
    for region in &input.regions {
        if !KNOWN.contains(&region.provider.as_str()) {
            return Err(MachinesError::Invalid(FieldError::new(
                "regions",
                ErrorCode::UnknownMachineProvider,
            )));
        }
        if !is_location(&region.location) {
            return Err(MachinesError::Invalid(FieldError::new(
                "regions",
                ErrorCode::InvalidMachineLocation,
            )));
        }
    }

    let mut tx = AuditedTx::begin(pool, context).await?;
    sqlx::query!(
        "UPDATE machine_settings SET api_url = $1, port_range = $2",
        api_url,
        port_range,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM machine_regions")
        .execute(&mut *tx)
        .await?;
    for region in &input.regions {
        sqlx::query!(
            "INSERT INTO machine_regions (region_id, provider, location) VALUES ($1, $2, $3)",
            region.region_id,
            region.provider,
            region.location,
        )
        .execute(&mut *tx)
        .await
        .map_err(|error| match error.as_database_error() {
            Some(db) if db.is_foreign_key_violation() => {
                MachinesError::Nodes(NodesError::UnknownRegion)
            }
            Some(db) if db.is_unique_violation() => MachinesError::Invalid(FieldError::new(
                "regions",
                ErrorCode::InvalidMachineLocation,
            )),
            _ => error.into(),
        })?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::MachinesSettingsUpdated)
            .target("machines", "settings")
            .details(json!({
                "api_url": api_url,
                "port_range": port_range,
                "regions": input.regions.iter().map(|r| json!({
                    "region_id": r.region_id, "provider": r.provider, "location": r.location,
                })).collect::<Vec<_>>(),
            })),
    )
    .await?;
    settings(pool).await
}

/// Fournisseur nommé, avec un jeton donné.
fn connect(name: &str, token: String) -> Result<Arc<dyn Provider>, MachinesError> {
    match name {
        "hetzner" => Ok(Arc::new(
            Hetzner::new(token).map_err(MachinesError::Internal)?,
        )),
        _ => Err(MachinesError::Unprocessable(
            ErrorCode::UnknownMachineProvider,
        )),
    }
}

/// Jeton enregistré : les réglages, et les régions créées pour les emplacements du fournisseur.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ProviderSaved {
    pub settings: MachinesSettings,
    /// Noms des régions créées (désactivées, à activer dans Infrastructure › Régions).
    pub created_regions: Vec<String>,
}

/// Enregistre les identifiants d'un fournisseur, après les avoir essayés (lecture des machines),
/// puis relie une région à chacun de ses emplacements.
pub async fn set_provider(
    state: &MachinesState,
    context: &AuditContext,
    name: &str,
    token: &str,
) -> Result<ProviderSaved, MachinesError> {
    if !KNOWN.contains(&name) {
        return Err(MachinesError::NotFound);
    }
    let token = token.trim();
    if token.is_empty() || token.len() > 500 {
        return Err(MachinesError::Invalid(FieldError::new(
            "token",
            ErrorCode::MachineProviderNotConfigured,
        )));
    }
    let tag = load(&state.pool).await?.instance_tag;
    let provider = match &state.fixed {
        Some(fixed) => fixed.clone(),
        None => connect(name, token.to_owned())?,
    };
    provider.list(LABEL, (SCOPE_LABEL, &tag)).await?;
    let sealed = secret_box::seal(&state.key, token.as_bytes())?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    sqlx::query!(
        "INSERT INTO machine_providers (provider, token) VALUES ($1, $2)
         ON CONFLICT (provider) DO UPDATE SET token = EXCLUDED.token, updated_at = now()",
        name,
        sealed,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit(AuditEvent::success(AuditAction::MachinesProviderUpdated).target("provider", name))
        .await?;
    let created_regions = sync_regions(state, context, name, provider.as_ref()).await?;
    Ok(ProviderSaved {
        settings: settings(&state.pool).await?,
        created_regions,
    })
}

/// Rejoue la création des régions des emplacements d'un fournisseur déjà configuré (jeton
/// enregistré avant cette fonction, nouvel emplacement chez le fournisseur).
pub async fn create_regions(
    state: &MachinesState,
    context: &AuditContext,
    name: &str,
) -> Result<ProviderSaved, MachinesError> {
    if !KNOWN.contains(&name) {
        return Err(MachinesError::NotFound);
    }
    let provider = state.provider(name).await?;
    let created_regions = sync_regions(state, context, name, provider.as_ref()).await?;
    Ok(ProviderSaved {
        settings: settings(&state.pool).await?,
        created_regions,
    })
}

/// Emplacements d'un fournisseur configuré.
pub async fn locations(state: &MachinesState, name: &str) -> Result<Vec<Location>, MachinesError> {
    if !KNOWN.contains(&name) {
        return Err(MachinesError::NotFound);
    }
    Ok(state.provider(name).await?.locations().await?)
}

/// Identifiant de région tiré d'une ville (`Ashburn, VA` → `ashburn-va`).
fn region_slug(city: &str) -> String {
    let mut slug = String::new();
    for c in city.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').chars().take(64).collect()
}

/// Une région par emplacement du fournisseur, nommée d'après sa ville et placée sur la carte :
/// créée **désactivée** si elle manque (rien n'est proposé sans décision, ADR-0053), puis reliée
/// à l'emplacement, à la place de toute autre région qui l'était. Rejouable ; les noms des
/// régions créées.
async fn sync_regions(
    state: &MachinesState,
    context: &AuditContext,
    name: &str,
    provider: &dyn Provider,
) -> Result<Vec<String>, MachinesError> {
    use crate::nodes::application::regions;
    let mut created = Vec::new();
    let mut linked = Vec::new();
    for location in provider.locations().await? {
        let slug = region_slug(&location.city);
        if slug.is_empty() || !is_location(&location.name) {
            continue;
        }
        let existing = {
            let mut conn = state.pool.acquire().await?;
            crate::nodes::infra::RegionRepository::id_by_slug(&mut conn, &slug).await?
        };
        let region_id = match existing {
            Some(id) => id,
            None => {
                let region_name = format!("{} ({})", location.city, location.country);
                let region = regions::create(
                    &state.pool,
                    context,
                    regions::NewRegion {
                        slug: slug.clone(),
                        name: region_name.clone(),
                        latitude: location.latitude,
                        longitude: location.longitude,
                    },
                )
                .await?;
                regions::update(
                    &state.pool,
                    context,
                    region.id,
                    regions::RegionPatch {
                        enabled: Some(false),
                        ..Default::default()
                    },
                )
                .await?;
                created.push(region_name);
                region.id
            }
        };
        linked.push((region_id, location.name));
    }

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    for (region_id, location) in &linked {
        // Une autre région reliée à cet emplacement en est détachée (ses nœuds ne changent pas).
        sqlx::query!(
            "DELETE FROM machine_regions WHERE provider = $1 AND location = $2 AND region_id <> $3",
            name,
            location,
            region_id,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO machine_regions (region_id, provider, location) VALUES ($1, $2, $3)
             ON CONFLICT (region_id) DO UPDATE
                SET provider = EXCLUDED.provider, location = EXCLUDED.location",
            region_id,
            name,
            location,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::MachinesSettingsUpdated)
            .target("provider", name)
            .details(json!({
                "regions_created": created,
                "locations": linked.iter().map(|(_, l)| l).collect::<Vec<_>>(),
            })),
    )
    .await?;
    Ok(created)
}

/// Retire les identifiants d'un fournisseur qu'aucune machine ni région n'utilise.
pub async fn remove_provider(
    pool: &PgPool,
    context: &AuditContext,
    name: &str,
) -> Result<MachinesSettings, MachinesError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let used: Option<bool> = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM machines WHERE provider = $1 AND state <> 'deleted')
             OR EXISTS (SELECT 1 FROM machine_regions WHERE provider = $1)",
        name,
    )
    .fetch_one(&mut *tx)
    .await?;
    if used == Some(true) {
        return Err(MachinesError::Conflict(ErrorCode::MachineProviderInUse));
    }
    let removed = sqlx::query!("DELETE FROM machine_providers WHERE provider = $1", name)
        .execute(&mut *tx)
        .await?;
    if removed.rows_affected() == 0 {
        return Err(MachinesError::NotFound);
    }
    tx.commit(AuditEvent::success(AuditAction::MachinesProviderRemoved).target("provider", name))
        .await?;
    settings(pool).await
}

impl MachinesState {
    /// Fournisseur nommé, avec ses identifiants enregistrés.
    pub async fn provider(&self, name: &str) -> Result<Arc<dyn Provider>, MachinesError> {
        if let Some(fixed) = &self.fixed {
            return Ok(fixed.clone());
        }
        let sealed = sqlx::query_scalar!(
            "SELECT token FROM machine_providers WHERE provider = $1",
            name
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(MachinesError::Conflict(
            ErrorCode::MachineProviderNotConfigured,
        ))?;
        let token = String::from_utf8(secret_box::open(&self.key, &sealed)?)
            .map_err(|e| MachinesError::Internal(e.to_string()))?;
        connect(name, token)
    }
}

/// Au moins un fournisseur a des identifiants.
pub async fn any_provider(pool: &PgPool) -> Result<bool, sqlx::Error> {
    Ok(
        sqlx::query_scalar!("SELECT EXISTS (SELECT 1 FROM machine_providers)")
            .fetch_one(pool)
            .await?
            .unwrap_or(false),
    )
}

struct Hosting {
    provider: String,
    location: String,
}

async fn hosting(pool: &PgPool, region_id: Uuid) -> Result<Hosting, MachinesError> {
    sqlx::query_as!(
        Hosting,
        "SELECT provider, location FROM machine_regions WHERE region_id = $1",
        region_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(MachinesError::Unprocessable(
        ErrorCode::RegionWithoutMachineProvider,
    ))
}

/// Offres du fournisseur de la région, de la moins chère à la plus chère.
pub async fn offers(state: &MachinesState, region_id: Uuid) -> Result<Vec<Offer>, MachinesError> {
    let hosting = hosting(&state.pool, region_id).await?;
    let provider = state.provider(&hosting.provider).await?;
    Ok(provider.offers(&hosting.location).await?)
}

// Lecture.

#[derive(Debug, Clone)]
struct MachineRow {
    id: Uuid,
    provider: String,
    reference: Option<String>,
    offer: String,
    location: String,
    node_id: Option<Uuid>,
    snapshot: Option<String>,
    claim: Option<String>,
    state: String,
    server_id: Option<Uuid>,
    flag: Option<String>,
    node_online_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl MachineRow {
    fn state(&self) -> MachineState {
        MachineState::parse(&self.state).unwrap_or(MachineState::Deleted)
    }
}

async fn row(pool: &PgPool, id: Uuid) -> Result<MachineRow, MachinesError> {
    sqlx::query_as!(
        MachineRow,
        "SELECT id, provider, reference, offer, location, node_id, snapshot, claim, state,
                server_id, flag, node_online_at, created_at, updated_at
           FROM machines WHERE id = $1 AND state <> 'deleted'",
        id
    )
    .fetch_optional(pool)
    .await?
    .ok_or(MachinesError::NotFound)
}

/// Machine vue par le panel et les extensions.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MachineView {
    pub id: Uuid,
    pub provider: String,
    pub reference: Option<String>,
    pub offer: String,
    pub location: String,
    pub region_id: Option<Uuid>,
    pub region_name: Option<String>,
    /// Coût horaire HT au moment de la location, en 1/100 000 d'euro.
    pub hourly_cost: i64,
    /// Coût mensuel plafonné HT, en centimes.
    pub monthly_cost: i64,
    pub node_id: Option<Uuid>,
    pub node_name: Option<String>,
    pub node_status: Option<String>,
    pub ipv4: Option<String>,
    /// Un snapshot est gardé (sommeil).
    pub has_snapshot: bool,
    pub claim: Option<String>,
    /// `creating`, `ready`, `sleeping`, `asleep`, `waking`, `releasing`, `deleted`.
    pub state: String,
    /// Serveur créé pour cette machine (il attend son nœud, puis s'installe dessus).
    pub server_id: Option<Uuid>,
    pub server_name: Option<String>,
    pub server_state: Option<String>,
    /// Signalement : `stalled`, `lost`.
    pub flag: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[schema(value_type = Option<String>, format = DateTime)]
    pub node_online_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    /// Installation : dernière nouvelle de chaque étape atteinte, dans l'ordre chronologique.
    #[schema(value_type = Vec<MachineProgress>)]
    pub progress: sqlx::types::Json<Vec<MachineProgress>>,
}

/// Étape de l'installation d'une machine louée (`vm_requested`, `vm_started`, `install_started`,
/// `enrolled`, `installed`, `online`).
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct MachineProgress {
    pub step: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub at: OffsetDateTime,
    /// Échec de l'étape : sortie courte de l'installeur, ou `no_news` (aucune nouvelle de la
    /// machine, souvent l'adresse de l'API injoignable depuis la VM).
    pub error: Option<String>,
}

/// Machines encore là, les plus récentes d'abord ; `claim` pour celles d'un appelant.
pub async fn list(pool: &PgPool, claim: Option<&str>) -> Result<Vec<MachineView>, MachinesError> {
    Ok(sqlx::query_as!(
        MachineView,
        r#"SELECT m.id, m.provider, m.reference, m.offer, m.location, m.region_id,
                  r.name AS "region_name?", m.hourly_cost, m.monthly_cost, m.node_id,
                  n.name AS "node_name?", n.status AS "node_status?", m.ipv4,
                  m.snapshot IS NOT NULL AS "has_snapshot!", m.claim, m.state,
                  m.server_id, s.name AS "server_name?", s.state AS "server_state?", m.flag,
                  m.node_online_at, m.created_at,
                  COALESCE((SELECT jsonb_agg(jsonb_build_object('step', p.step, 'at', p.at,
                                                                'error', p.error) ORDER BY p.at)
                              FROM machine_progress p WHERE p.machine_id = m.id), '[]')
                    AS "progress!: sqlx::types::Json<Vec<MachineProgress>>"
             FROM machines m
             LEFT JOIN regions r ON r.id = m.region_id
             LEFT JOIN nodes n ON n.id = m.node_id
             LEFT JOIN servers s ON s.id = m.server_id
            WHERE m.state <> 'deleted' AND ($1::text IS NULL OR m.claim = $1)
            ORDER BY m.created_at DESC"#,
        claim,
    )
    .fetch_all(pool)
    .await?)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<MachineView, MachinesError> {
    list(pool, None)
        .await?
        .into_iter()
        .find(|machine| machine.id == id)
        .ok_or(MachinesError::NotFound)
}

/// Machines d'un appelant (`cloud:subscription:<id>`).
pub async fn by_claim(pool: &PgPool, claim: &str) -> Result<Vec<MachineView>, MachinesError> {
    list(pool, Some(claim)).await
}

// Location.

/// Serveur à créer sur la machine : il attend son nœud (`waiting_node`), puis s'y installe.
#[derive(Debug, Clone)]
pub struct PendingServer {
    pub name: String,
    pub prefab_id: String,
    pub prefab_version: Option<String>,
    pub memory_mb: Option<i32>,
    pub cpu_percent: Option<i32>,
    pub disk_mb: Option<i32>,
    pub settings: Map<String, Value>,
    pub start_when_installed: bool,
    pub backup_limit: Option<i32>,
    pub owner_ref: Option<String>,
}

/// Offre nommée, ou la moins chère qui couvre des ressources.
#[derive(Debug, Clone)]
pub enum OfferChoice {
    Named(String),
    Covering(Resources),
}

#[derive(Debug, Clone)]
pub struct RentInput {
    pub region_id: Uuid,
    pub offer: OfferChoice,
    /// Appelant (extension) ; une machine réclamée a un nœud réservé.
    pub claim: Option<String>,
    pub pending_server: Option<PendingServer>,
}

/// Loue une machine et l'enrôle comme nœud. La ligne est écrite avant l'appel au fournisseur :
/// une réponse perdue laisse une ligne `creating`, rattachée par le passage suivant.
pub async fn rent(
    state: &MachinesState,
    context: &AuditContext,
    input: RentInput,
) -> Result<MachineView, MachinesError> {
    let claim = input
        .claim
        .map(|claim| claim.trim().to_owned())
        .filter(|claim| !claim.is_empty());
    if claim
        .as_deref()
        .is_some_and(|claim| !crate::validation::is_display_text(claim, 1, 200))
    {
        return Err(MachinesError::Invalid(FieldError::new(
            "claim",
            ErrorCode::InvalidClaim,
        )));
    }
    let pool = &state.pool;
    let settings = load(pool).await?;
    let api_url = settings
        .api_url
        .clone()
        .ok_or(MachinesError::Conflict(ErrorCode::MachinesNotConfigured))?;
    let hosting = hosting(pool, input.region_id).await?;
    let provider = state.provider(&hosting.provider).await?;
    let offers = provider.offers(&hosting.location).await?;
    let offer = match &input.offer {
        OfferChoice::Named(name) => offers.iter().find(|offer| offer.name == *name),
        OfferChoice::Covering(need) => cheapest_covering(&offers, *need),
    }
    .cloned()
    .ok_or(MachinesError::Invalid(FieldError::new(
        "offer",
        ErrorCode::UnknownMachineOffer,
    )))?;
    // Le serveur attendu prend la machine entière (réserve du système déduite) pour ce qu'il ne
    // fixe pas ; ce qu'il fixe doit y tenir.
    let mut pending_server = input.pending_server;
    if let Some(server) = &mut pending_server {
        let cores = i32::try_from(offer.vcpus)
            .unwrap_or(i32::MAX)
            .saturating_mul(100);
        let memory = i32::try_from(
            offer
                .memory_mb
                .saturating_sub(memory_reserve_mb(offer.memory_mb)),
        )
        .unwrap_or(0);
        let disk = i32::try_from(offer.disk_gb.saturating_sub(DISK_RESERVE_GB))
            .unwrap_or(0)
            .saturating_mul(1024);
        server.cpu_percent.get_or_insert(cores);
        server.memory_mb.get_or_insert(memory);
        server.disk_mb.get_or_insert(disk);
        // Réserve du système déduite, il ne reste rien pour le serveur (offre de 1 Go…).
        if server
            .memory_mb
            .is_some_and(|m| !crate::servers::application::MEMORY_MB.contains(&m))
        {
            return Err(too_small());
        }
    }
    if let Some(server) = &pending_server {
        let platform = crate::settings::application::get(pool)
            .await
            .map_err(|e| MachinesError::Internal(e.to_string()))?;
        let need = Resources {
            memory_mb: server.memory_mb.unwrap_or(platform.default_memory_mb),
            cpu_percent: server.cpu_percent.unwrap_or(platform.default_cpu_percent),
            disk_mb: server.disk_mb.unwrap_or(platform.default_disk_mb),
        };
        if cheapest_covering(std::slice::from_ref(&offer), need).is_none() {
            return Err(too_small());
        }
    }
    let id = Uuid::now_v7();
    let name = machine_name(id);
    let ticket = add_node(
        pool,
        context,
        &name,
        PlacementInput {
            region_id: Some(input.region_id),
            ..Default::default()
        },
        ENROLL_TTL,
    )
    .await?;
    let node_id = ticket.node.id;
    // Ports des serveurs sur toutes les adresses de la machine.
    if let Err(error) =
        allocations::create(pool, context, node_id, None, &settings.port_range).await
    {
        forget_node(state, context, node_id).await;
        return Err(error.into());
    }

    let mut tx = AuditedTx::begin(pool, context).await?;
    if claim.is_some() {
        NodeRepository::set_reserved(&mut tx, node_id, true).await?;
    }
    sqlx::query!(
        "INSERT INTO machines (id, provider, offer, location, region_id, hourly_cost,
                               monthly_cost, node_id, claim, state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'creating')",
        id,
        hosting.provider,
        offer.name,
        hosting.location,
        input.region_id,
        offer.hourly_cost,
        offer.monthly_cost,
        node_id,
        claim,
    )
    .execute(&mut *tx)
    .await?;
    record_step(&mut tx, id, "vm_requested").await?;
    tx.commit(
        AuditEvent::success(AuditAction::MachineRented)
            .target("machine", id)
            .details(json!({
                "provider": hosting.provider,
                "offer": offer.name,
                "location": hosting.location,
                "region_id": input.region_id,
                "node_id": node_id,
                "claim": claim,
                "hourly_cost": offer.hourly_cost,
                "server": pending_server.as_ref().map(|server| &server.name),
            })),
    )
    .await?;

    // Le serveur existe tout de suite, en attente de son nœud : son installation se suit comme
    // celle de tout serveur. Ses réglages sont vérifiés avant de louer quoi que ce soit.
    let server = match pending_server {
        Some(server) => match waiting_server(state, context, id, node_id, server).await {
            Ok(server) => Some(server),
            Err(error) => {
                abandon(state, context, &row(pool, id).await?, "server refused").await?;
                // Ressources prises sur l'offre : c'est elle qui ne suffit pas.
                return Err(match error {
                    MachinesError::Server(crate::servers::application::ServersError::Invalid(
                        field,
                    )) if ["memory_mb", "cpu_percent", "disk_mb"].contains(&&*field.field) => {
                        too_small()
                    }
                    other => other,
                });
            }
        },
        None => None,
    };

    let spec = MachineSpec {
        name,
        offer: offer.name.clone(),
        location: hosting.location.clone(),
        user_data: cloud_init(
            &api_url,
            &state.ca_fingerprint,
            ticket.token.expose(),
            &settings.port_range,
        ),
        labels: labels(id, &settings.instance_tag),
        snapshot: None,
    };
    match provider.create(spec).await {
        Ok(machine) => {
            let listed = Listed {
                reference: machine.reference,
                label: id.to_string(),
                ipv4: machine.ipv4,
                created_at: OffsetDateTime::now_utc(),
            };
            adopt(state, context, &row(pool, id).await?, &listed).await?;
        }
        Err(error @ ProviderError::Refused { .. }) => {
            if let Some(server) = server {
                discard_server(state, context, server).await;
            }
            abandon(state, context, &row(pool, id).await?, &error.to_string()).await?;
            return Err(error.into());
        }
        // Réponse perdue : la ligne reste `creating`, le passage rattache la VM ou abandonne.
        Err(error) => tracing::warn!(%error, machine = %id, "machine creation answer lost"),
    }
    get(pool, id).await
}

fn too_small() -> MachinesError {
    MachinesError::Invalid(FieldError::new("offer", ErrorCode::MachineOfferTooSmall))
}

/// Serveur créé sur le nœud (encore en attente) de la machine, noté sur elle.
async fn waiting_server(
    state: &MachinesState,
    context: &AuditContext,
    machine: Uuid,
    node_id: Uuid,
    server: PendingServer,
) -> Result<Uuid, MachinesError> {
    let created = crate::servers::application::create_waiting(
        &state.servers,
        context,
        NewServer {
            name: server.name,
            node_id,
            prefab_id: server.prefab_id,
            prefab_version: server.prefab_version,
            memory_mb: server.memory_mb,
            cpu_percent: server.cpu_percent,
            disk_mb: server.disk_mb,
            settings: server.settings,
            allocations: BTreeMap::new(),
            start_when_installed: server.start_when_installed,
            backup_limit: server.backup_limit,
            owner_ref: server.owner_ref,
        },
    )
    .await
    .map_err(MachinesError::Server)?;
    let id = created.server.id;
    sqlx::query!(
        "UPDATE machines SET server_id = $2 WHERE id = $1",
        machine,
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(id)
}

/// Machine refusée avant d'exister : son serveur, jamais déployé, disparaît avec elle.
async fn discard_server(state: &MachinesState, context: &AuditContext, server: Uuid) {
    if let Err(error) = crate::servers::application::delete(&state.servers, context, server).await {
        tracing::warn!(%error, %server, "server of a refused machine kept");
    }
}

/// Machine perdue avant l'arrivée de son nœud : son serveur passe en échec, avec la raison.
async fn fail_server(
    pool: &PgPool,
    row: &MachineRow,
    reason: ErrorCode,
) -> Result<(), sqlx::Error> {
    if let Some(server) = row.server_id {
        sqlx::query!(
            "UPDATE servers SET state = 'failed', state_reason = $2, updated_at = now()
              WHERE id = $1 AND state = 'waiting_node'",
            server,
            reason.as_str(),
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn labels(id: Uuid, tag: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (LABEL.to_owned(), id.to_string()),
        (SCOPE_LABEL.to_owned(), tag.to_owned()),
    ])
}

/// VM trouvée (ou créée) pour une ligne `creating` ou `waking` : référence et adresse notées.
async fn adopt(
    state: &MachinesState,
    context: &AuditContext,
    row: &MachineRow,
    vm: &Listed,
) -> Result<(), MachinesError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let adopted = sqlx::query!(
        "UPDATE machines
            SET reference = $2, ipv4 = COALESCE($3, ipv4),
                state = CASE WHEN state = 'creating' THEN 'ready' ELSE state END,
                updated_at = CASE WHEN state = 'creating' THEN now() ELSE updated_at END
          WHERE id = $1 AND state IN ('creating', 'waking')",
        row.id,
        vm.reference,
        vm.ipv4,
    )
    .execute(&mut *tx)
    .await?;
    if adopted.rows_affected() == 0 {
        return Ok(());
    }
    record_step(&mut tx, row.id, "vm_started").await?;
    // Adresse publique du nœud (ports à donner aux joueurs, DNS) : celle de la VM, qui change à
    // chaque réveil.
    if let (Some(node), Some(ipv4)) = (row.node_id, &vm.ipv4) {
        sqlx::query!(
            "UPDATE nodes SET public_ipv4 = $2, updated_at = now() WHERE id = $1",
            node,
            ipv4,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::MachineReady)
            .target("machine", row.id)
            .details(json!({ "reference": vm.reference, "ipv4": vm.ipv4, "state": row.state })),
    )
    .await?;
    if row.state() == MachineState::Creating {
        emit(state, row, MachineEventKind::Ready);
    }
    Ok(())
}

/// `creating` sans VM : nœud retiré, ligne close. `waking` sans VM : retour au sommeil.
async fn abandon(
    state: &MachinesState,
    context: &AuditContext,
    row: &MachineRow,
    reason: &str,
) -> Result<(), MachinesError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let changed = sqlx::query!(
        "UPDATE machines
            SET state = CASE WHEN state = 'waking' THEN 'asleep' ELSE 'deleted' END,
                deleted_at = CASE WHEN state = 'waking' THEN NULL ELSE now() END,
                reference = CASE WHEN state = 'waking' THEN NULL ELSE reference END,
                updated_at = now()
          WHERE id = $1 AND state IN ('creating', 'waking')",
        row.id,
    )
    .execute(&mut *tx)
    .await?;
    if changed.rows_affected() == 0 {
        return Ok(());
    }
    tx.commit(
        AuditEvent::failure(AuditAction::MachineAbandoned)
            .target("machine", row.id)
            .details(json!({ "reason": reason, "state": row.state })),
    )
    .await?;
    if row.state() == MachineState::Creating
        && let Some(node) = row.node_id
    {
        forget_node(state, context, node).await;
    }
    Ok(())
}

/// Nœud d'une machine rendue : révoqué puis supprimé. Une étape déjà faite n'est pas une erreur.
async fn forget_node(state: &MachinesState, context: &AuditContext, node: Uuid) {
    let pool = &state.pool;
    match lifecycle::revoke(pool, context, state.links.as_ref(), node, None).await {
        Ok(_) | Err(NodesError::NotFound | NodesError::Conflict(_)) => {}
        Err(error) => tracing::warn!(%error, %node, "machine node not revoked"),
    }
    match lifecycle::delete(pool, context, state.links.as_ref(), node).await {
        Ok(()) | Err(NodesError::NotFound) => {}
        Err(error) => tracing::warn!(%error, %node, "machine node kept"),
    }
}

fn emit(state: &MachinesState, row: &MachineRow, kind: MachineEventKind) {
    state.events.emit(MachineEvent {
        machine_id: row.id,
        node_id: row.node_id,
        claim: row.claim.clone(),
        kind,
    });
}

// Retour.

/// Rend une machine : VM et snapshot supprimés chez le fournisseur, nœud supprimé. Rejouable.
pub async fn release(
    state: &MachinesState,
    context: &AuditContext,
    id: Uuid,
) -> Result<(), MachinesError> {
    let row = row(&state.pool, id).await?;
    match row.state() {
        MachineState::Creating => return Err(MachinesError::Conflict(ErrorCode::MachineCreating)),
        MachineState::Sleeping | MachineState::Waking => {
            return Err(MachinesError::Conflict(ErrorCode::MachineBusy));
        }
        MachineState::Ready | MachineState::Asleep | MachineState::Releasing => {}
        MachineState::Deleted => return Err(MachinesError::NotFound),
    }
    if let Some(node) = row.node_id {
        let mut conn = state.pool.acquire().await?;
        if NodeRepository::count_servers(&mut conn, node).await? > 0 {
            return Err(MachinesError::Conflict(ErrorCode::NodeHasServers));
        }
    }
    mark_releasing(&state.pool, context, &row).await?;
    finish_release(state, context, &row).await
}

async fn mark_releasing(
    pool: &PgPool,
    context: &AuditContext,
    row: &MachineRow,
) -> Result<(), MachinesError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    sqlx::query!(
        "UPDATE machines SET state = 'releasing', updated_at = now()
          WHERE id = $1 AND state IN ('ready', 'asleep', 'releasing')",
        row.id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::MachineReleased)
            .target("machine", row.id)
            .details(json!({ "previous_state": row.state, "reference": row.reference })),
    )
    .await?;
    Ok(())
}

async fn finish_release(
    state: &MachinesState,
    context: &AuditContext,
    row: &MachineRow,
) -> Result<(), MachinesError> {
    let provider = state.provider(&row.provider).await?;
    // Nœud coupé d'abord : un serveur ne peut plus y être placé pendant le retour.
    if let Some(node) = row.node_id {
        match lifecycle::revoke(&state.pool, context, state.links.as_ref(), node, None).await {
            Ok(_) | Err(NodesError::NotFound | NodesError::Conflict(_)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(reference) = &row.reference {
        provider.delete(reference).await?;
    }
    if let Some(snapshot) = &row.snapshot {
        provider.delete_snapshot(snapshot).await?;
    }
    if let Some(node) = row.node_id {
        match lifecycle::delete(&state.pool, context, state.links.as_ref(), node).await {
            Ok(()) | Err(NodesError::NotFound) => {}
            // Son serveur en échec y est encore : le nœud reste, à supprimer avec lui.
            Err(NodesError::Conflict(ErrorCode::NodeHasServers)) => {
                tracing::warn!(%node, "released machine node kept: a server is still on it");
            }
            Err(error) => return Err(error.into()),
        }
    }
    let done = sqlx::query!(
        "UPDATE machines SET state = 'deleted', deleted_at = now(), updated_at = now()
          WHERE id = $1 AND state = 'releasing'",
        row.id,
    )
    .execute(&state.pool)
    .await?;
    if done.rows_affected() > 0 {
        emit(state, row, MachineEventKind::Released);
    }
    Ok(())
}

// Sommeil et réveil.

/// Demande le snapshot ; la VM n'est rendue qu'une fois le snapshot prêt (passage suivant, ou
/// tout de suite s'il l'est déjà).
pub async fn sleep(
    state: &MachinesState,
    context: &AuditContext,
    id: Uuid,
) -> Result<MachineView, MachinesError> {
    let row = row(&state.pool, id).await?;
    if row.state() != MachineState::Ready {
        return Err(MachinesError::Conflict(ErrorCode::MachineNotReady));
    }
    // Un serveur attend encore ce nœud : il serait déployé sur une machine endormie.
    let waiting: Option<bool> = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM servers WHERE node_id = $1 AND state = 'waiting_node')",
        row.node_id,
    )
    .fetch_one(&state.pool)
    .await?;
    if waiting == Some(true) {
        return Err(MachinesError::Conflict(ErrorCode::MachineSleepUnsupported));
    }
    let reference = row
        .reference
        .clone()
        .ok_or(MachinesError::Conflict(ErrorCode::MachineNotReady))?;
    let tag = load(&state.pool).await?.instance_tag;
    let provider = state.provider(&row.provider).await?;
    let snapshot = provider.snapshot(&reference, &labels(id, &tag)).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    sqlx::query!(
        "UPDATE machines SET state = 'sleeping', snapshot = $2, updated_at = now()
          WHERE id = $1 AND state = 'ready'",
        id,
        snapshot,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit(
        AuditEvent::success(AuditAction::MachineSleepRequested)
            .target("machine", id)
            .details(json!({ "snapshot": snapshot })),
    )
    .await?;
    let row = self::row(&state.pool, id).await?;
    advance_sleep(state, provider.as_ref(), &row).await?;
    get(&state.pool, id).await
}

/// Snapshot prêt : VM rendue, machine endormie. Snapshot disparu : retour à `ready`.
async fn advance_sleep(
    state: &MachinesState,
    provider: &dyn Provider,
    row: &MachineRow,
) -> Result<bool, MachinesError> {
    let Some(snapshot) = &row.snapshot else {
        return Ok(false);
    };
    match provider.snapshot_state(snapshot).await? {
        SnapshotState::Pending => Ok(false),
        SnapshotState::Missing => {
            tracing::error!(machine = %row.id, "snapshot gone before the machine slept");
            sqlx::query!(
                "UPDATE machines SET state = 'ready', snapshot = NULL, updated_at = now()
                  WHERE id = $1 AND state = 'sleeping'",
                row.id,
            )
            .execute(&state.pool)
            .await?;
            Ok(false)
        }
        SnapshotState::Ready => {
            if let Some(reference) = &row.reference {
                provider.delete(reference).await?;
            }
            let context = machine_context(row.id);
            let mut tx = AuditedTx::begin(&state.pool, &context).await?;
            let done = sqlx::query!(
                "UPDATE machines
                    SET state = 'asleep', reference = NULL, ipv4 = NULL, updated_at = now()
                  WHERE id = $1 AND state = 'sleeping'",
                row.id,
            )
            .execute(&mut *tx)
            .await?;
            if done.rows_affected() == 0 {
                return Ok(false);
            }
            tx.commit(AuditEvent::success(AuditAction::MachineAsleep).target("machine", row.id))
                .await?;
            emit(state, row, MachineEventKind::Asleep);
            Ok(true)
        }
    }
}

/// Recrée la VM depuis son snapshot ; le nœud se reconnecte avec tout son disque, puis le
/// snapshot est supprimé (passage suivant). L'adresse IP change.
pub async fn wake(
    state: &MachinesState,
    context: &AuditContext,
    id: Uuid,
) -> Result<MachineView, MachinesError> {
    let row = row(&state.pool, id).await?;
    if row.state() != MachineState::Asleep {
        return Err(MachinesError::Conflict(ErrorCode::MachineNotAsleep));
    }
    let snapshot = row
        .snapshot
        .clone()
        .ok_or(MachinesError::Conflict(ErrorCode::MachineNotAsleep))?;
    let tag = load(&state.pool).await?.instance_tag;
    let provider = state.provider(&row.provider).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    sqlx::query!(
        "UPDATE machines SET state = 'waking', updated_at = now()
          WHERE id = $1 AND state = 'asleep'",
        id,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit(AuditEvent::success(AuditAction::MachineWakeRequested).target("machine", id))
        .await?;
    let spec = MachineSpec {
        name: machine_name(id),
        offer: row.offer.clone(),
        location: row.location.clone(),
        user_data: NOTHING_TO_INIT.to_owned(),
        labels: labels(id, &tag),
        snapshot: Some(snapshot),
    };
    let waking = self::row(&state.pool, id).await?;
    match provider.create(spec).await {
        Ok(machine) => {
            let listed = Listed {
                reference: machine.reference,
                label: id.to_string(),
                ipv4: machine.ipv4,
                created_at: OffsetDateTime::now_utc(),
            };
            adopt(state, context, &waking, &listed).await?;
        }
        Err(error @ ProviderError::Refused { .. }) => {
            abandon(state, context, &waking, &error.to_string()).await?;
            return Err(error.into());
        }
        Err(error) => tracing::warn!(%error, machine = %id, "machine wake answer lost"),
    }
    get(&state.pool, id).await
}

/// Acteur des actions faites par le ménage pour une machine.
fn machine_context(id: Uuid) -> AuditContext {
    AuditContext::system().with_actor(Actor {
        kind: ActorKind::System,
        id: Some(id),
    })
}

// Passage de ménage.

/// Rapport d'un passage ; en simulation, ce qui aurait été fait pour les décisions
/// destructives (abandon, orphelines, machines bloquées rendues). Les opérations demandées
/// (rattacher, finir un sommeil ou un réveil, créer le serveur attendu) se font toujours.
#[derive(Debug, Default, Serialize, PartialEq, Eq)]
pub struct SweepReport {
    pub dry_run: bool,
    pub adopted: u32,
    pub abandoned: u32,
    pub orphans: u32,
    pub lost: u32,
    pub stalled: u32,
    pub released: u32,
    pub asleep: u32,
    pub awake: u32,
    pub nodes_online: u32,
    pub errors: u32,
}

/// La tâche `machines.sweep` (ADR-0062).
pub fn task(state: MachinesState) -> Task {
    Task::new("machines", SWEEP, None, move |context| {
        let state = state.clone();
        Box::pin(async move {
            let report = sweep(&state, context.now, context.dry_run)
                .await
                .map_err(|e| TaskError(e.to_string()))?;
            serde_json::to_value(report).map_err(|e| TaskError(e.to_string()))
        })
    })
}

async fn node_online(pool: &PgPool, node: Option<Uuid>) -> Result<bool, sqlx::Error> {
    let Some(node) = node else {
        return Ok(false);
    };
    let mut conn = pool.acquire().await?;
    Ok(NodeRepository::get(&mut conn, node)
        .await?
        .is_some_and(|n| n.status.parse::<NodeStatus>().ok() == Some(NodeStatus::Online)))
}

/// Un passage : chaque fournisseur utilisé est relu (VM de cette instance), comparé à
/// `machines`, puis chaque action appliquée. Rejouable : chaque action vérifie l'état qu'elle
/// change.
pub async fn sweep(
    state: &MachinesState,
    now: OffsetDateTime,
    dry_run: bool,
) -> Result<SweepReport, MachinesError> {
    let pool = &state.pool;
    let mut report = SweepReport {
        dry_run,
        ..Default::default()
    };
    let tag = load(pool).await?.instance_tag;
    let providers: Vec<Option<String>> = sqlx::query_scalar!(
        "SELECT provider FROM machine_providers
         UNION SELECT provider FROM machines WHERE state <> 'deleted'"
    )
    .fetch_all(pool)
    .await?;
    for name in providers.into_iter().flatten() {
        let (provider, vms) = match state.provider(&name).await {
            Ok(provider) => match provider.list(LABEL, (SCOPE_LABEL, &tag)).await {
                Ok(vms) => (provider, vms),
                Err(error) => {
                    tracing::error!(%error, provider = %name, "machines sweep: listing failed");
                    report.errors += 1;
                    continue;
                }
            },
            Err(error) => {
                tracing::error!(%error, provider = %name, "machines sweep: provider unavailable");
                report.errors += 1;
                continue;
            }
        };
        let rows = sqlx::query_as!(
            MachineRow,
            "SELECT id, provider, reference, offer, location, node_id, snapshot, claim, state,
                    server_id, flag, node_online_at, created_at, updated_at
               FROM machines WHERE provider = $1 AND state <> 'deleted'",
            name,
        )
        .fetch_all(pool)
        .await?;

        for row in &rows {
            let vm = vms.iter().find(|vm| vm.label == row.id.to_string());
            if let Err(error) =
                machine_pass(state, provider.as_ref(), row, vm, now, dry_run, &mut report).await
            {
                tracing::error!(%error, machine = %row.id, "machines sweep: action failed");
                report.errors += 1;
            }
        }

        for vm in &vms {
            if rows.iter().any(|row| row.id.to_string() == vm.label) {
                continue;
            }
            if machine_step(None, Some(vm.created_at), now) != Action::Orphan {
                continue;
            }
            report.orphans += 1;
            if dry_run {
                continue;
            }
            if let Err(error) = delete_orphan(pool, provider.as_ref(), &name, vm).await {
                tracing::error!(%error, reference = %vm.reference, "machines sweep: orphan kept");
                report.errors += 1;
            }
        }
    }
    Ok(report)
}

async fn machine_pass(
    state: &MachinesState,
    provider: &dyn Provider,
    row: &MachineRow,
    vm: Option<&Listed>,
    now: OffsetDateTime,
    dry_run: bool,
    report: &mut SweepReport,
) -> Result<(), MachinesError> {
    let pool = &state.pool;
    let context = machine_context(row.id);
    let row_state = RowState {
        state: row.state(),
        created_at: row.created_at,
        since: row.updated_at,
        node_seen: row.node_online_at.is_some(),
        has_reference: row.reference.is_some(),
    };
    match machine_step(Some(row_state), vm.map(|vm| vm.created_at), now) {
        Action::Adopt => {
            report.adopted += 1;
            if let Some(vm) = vm {
                adopt(state, &context, row, vm).await?;
            }
        }
        Action::Abandon => {
            report.abandoned += 1;
            if !dry_run {
                fail_server(pool, row, ErrorCode::MachineUnavailable).await?;
                abandon(state, &context, row, "no machine at the provider").await?;
            }
        }
        Action::Lost => {
            report.lost += 1;
            flag(state, row, "lost", MachineEventKind::Lost).await?;
        }
        Action::Stalled => {
            report.stalled += 1;
            if row.claim.is_some() {
                flag(state, row, "stalled", MachineEventKind::Stalled).await?;
            } else if !dry_run {
                fail_server(pool, row, ErrorCode::MachineNotReadyInTime).await?;
                mark_releasing(pool, &context, row).await?;
                finish_release(state, &context, row).await?;
                report.released += 1;
            }
        }
        Action::FinishRelease => {
            finish_release(state, &context, row).await?;
            report.released += 1;
        }
        Action::Orphan | Action::Keep => match row.state() {
            MachineState::Ready if row.node_online_at.is_none() => {
                record_enrolled(pool, row).await?;
                if node_online(pool, row.node_id).await? {
                    report.nodes_online += 1;
                    node_came_online(state, row).await?;
                } else {
                    flag_silence(pool, row.id, now).await?;
                }
            }
            MachineState::Sleeping => {
                if advance_sleep(state, provider, row).await? {
                    report.asleep += 1;
                }
            }
            MachineState::Waking => {
                let woke = row.reference.is_some()
                    && node_online(pool, row.node_id).await?
                    && finish_wake(state, provider, row).await?;
                report.awake += u32::from(woke);
            }
            _ => {}
        },
    }
    Ok(())
}

/// Signalement noté une fois : l'événement n'est pas répété à chaque passage.
async fn flag(
    state: &MachinesState,
    row: &MachineRow,
    flag: &str,
    kind: MachineEventKind,
) -> Result<(), MachinesError> {
    if row.flag.as_deref() == Some(flag) {
        return Ok(());
    }
    tracing::warn!(machine = %row.id, flag, "machine flagged");
    sqlx::query!("UPDATE machines SET flag = $2 WHERE id = $1", row.id, flag)
        .execute(&state.pool)
        .await?;
    emit(state, row, kind);
    Ok(())
}

/// Premier passage avec le nœud en ligne : noté et signalé. Son serveur, s'il en a un, est
/// déployé par Kuro dès la connexion du nœud.
async fn node_came_online(state: &MachinesState, row: &MachineRow) -> Result<(), MachinesError> {
    let marked = sqlx::query!(
        "UPDATE machines SET node_online_at = now(), flag = NULL
          WHERE id = $1 AND node_online_at IS NULL",
        row.id,
    )
    .execute(&state.pool)
    .await?;
    if marked.rows_affected() > 0 {
        let mut conn = state.pool.acquire().await?;
        record_step(&mut conn, row.id, "online").await?;
        emit(state, row, MachineEventKind::NodeOnline);
    }
    Ok(())
}

// Installation (étapes).

/// Étape atteinte, notée une fois ; une erreur notée avant sur elle est effacée.
async fn record_step(
    conn: &mut sqlx::PgConnection,
    machine: Uuid,
    step: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO machine_progress (machine_id, step) VALUES ($1, $2)
         ON CONFLICT (machine_id, step) DO UPDATE SET at = now(), error = NULL
          WHERE machine_progress.error IS NOT NULL",
        machine,
        step,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Inscription reçue : le jeton d'enrôlement du nœud a servi (date de son usage).
async fn record_enrolled(pool: &PgPool, row: &MachineRow) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO machine_progress (machine_id, step, at)
         SELECT $1, 'enrolled', max(used_at) FROM enrollment_tokens
          WHERE node_id = $2 AND used_at IS NOT NULL HAVING max(used_at) IS NOT NULL
         ON CONFLICT (machine_id, step) DO UPDATE SET at = EXCLUDED.at, error = NULL
          WHERE machine_progress.error IS NOT NULL",
        row.id,
        row.node_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Installation muette depuis `NO_NEWS` : l'étape attendue passe en erreur `no_news`.
async fn flag_silence(
    pool: &PgPool,
    machine: Uuid,
    now: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    let progress: Vec<(String, OffsetDateTime, bool)> = sqlx::query!(
        r#"SELECT step, at, error IS NOT NULL AS "failed!" FROM machine_progress
            WHERE machine_id = $1"#,
        machine,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| (row.step, row.at, row.failed))
    .collect();
    if let Some(step) = silent_step(&progress, now) {
        tracing::warn!(%machine, step, "machine installation silent");
        sqlx::query!(
            "INSERT INTO machine_progress (machine_id, step, error) VALUES ($1, $2, $3)
             ON CONFLICT (machine_id, step) DO NOTHING",
            machine,
            step,
            NO_NEWS_ERROR,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Annonce de l'installeur d'une machine louée, authentifiée par le jeton d'enrôlement de son
/// nœud (encore valide, même déjà utilisé : l'installation continue après l'inscription).
/// `false` : jeton inconnu ou expiré, ou nœud sans machine.
pub async fn announce(
    pool: &PgPool,
    token: &str,
    step: &str,
    error: Option<&str>,
) -> Result<bool, MachinesError> {
    if !ANNOUNCED_STEPS.contains(&step) {
        return Err(MachinesError::Invalid(FieldError::new(
            "step",
            ErrorCode::InvalidParameter,
        )));
    }
    let hash = crate::enrollment::domain::TokenHash::of(token);
    let machine = sqlx::query_scalar!(
        "SELECT m.id FROM enrollment_tokens t
           JOIN machines m ON m.node_id = t.node_id AND m.state <> 'deleted'
          WHERE t.token_hash = $1 AND t.expires_at > now()",
        hash.as_str(),
    )
    .fetch_optional(pool)
    .await?;
    let Some(machine) = machine else {
        return Ok(false);
    };
    sqlx::query!(
        "INSERT INTO machine_progress (machine_id, step, error) VALUES ($1, $2, $3)
         ON CONFLICT (machine_id, step) DO UPDATE SET at = now(), error = EXCLUDED.error",
        machine,
        step,
        error.and_then(short_error),
    )
    .execute(pool)
    .await?;
    Ok(true)
}

/// VM recréée et nœud revenu : snapshot supprimé, machine prête.
async fn finish_wake(
    state: &MachinesState,
    provider: &dyn Provider,
    row: &MachineRow,
) -> Result<bool, MachinesError> {
    if let Some(snapshot) = &row.snapshot {
        provider.delete_snapshot(snapshot).await?;
    }
    let context = machine_context(row.id);
    let mut tx = AuditedTx::begin(&state.pool, &context).await?;
    let done = sqlx::query!(
        "UPDATE machines SET state = 'ready', snapshot = NULL, updated_at = now()
          WHERE id = $1 AND state = 'waking'",
        row.id,
    )
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    tx.commit(AuditEvent::success(AuditAction::MachineAwake).target("machine", row.id))
        .await?;
    emit(state, row, MachineEventKind::Awake);
    Ok(true)
}

async fn delete_orphan(
    pool: &PgPool,
    provider: &dyn Provider,
    name: &str,
    vm: &Listed,
) -> Result<(), MachinesError> {
    provider.delete(&vm.reference).await?;
    let context = AuditContext::system();
    let tx = AuditedTx::begin(pool, &context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::MachineOrphanDeleted)
            .target("machine", &vm.label)
            .details(json!({ "provider": name, "reference": vm.reference })),
    )
    .await?;
    Ok(())
}
