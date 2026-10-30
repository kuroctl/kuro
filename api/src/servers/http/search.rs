//! Liste paginée des serveurs et actions groupées (ADR-0025).

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::servers::{ServerDto, dto};
use crate::error_code::{ErrorCode, FieldError};
use crate::http::error::{ApiError, Problem};
use crate::http::extract::{Json, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::servers::application::ServersState;
use crate::servers::application::bulk::{self, BulkAction, MAX_SERVERS};
use crate::servers::application::search::{self, FacetValue, Filter, Search, SortKey};
use kuro_protocol::api::server::PowerAction;

/// Critères de la liste. Valeurs multiples séparées par des virgules (`prefab=a,b`).
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// Nom, identifiant, nœud ou prefab.
    pub q: Option<String>,
    pub prefab: Option<String>,
    /// Identifiants de nœuds.
    pub node: Option<String>,
    pub state: Option<String>,
    /// `true` : une version plus récente du prefab existe ; `false` : à jour.
    pub upgrade: Option<bool>,
    pub suspended: Option<bool>,
    /// `name`, `state`, `prefab`, `node`, `memory`, `cpu`, `disk`, `created`.
    pub sort: Option<String>,
    /// `asc` (défaut) ou `desc`.
    pub order: Option<String>,
    /// À partir de 1.
    pub page: Option<usize>,
    /// 1 à 100, 25 par défaut.
    pub per_page: Option<usize>,
    /// Rendre aussi les identifiants de tous les résultats (1000 au plus).
    #[serde(default)]
    pub all_ids: bool,
}

fn list(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .collect()
}

fn invalid(field: &'static str) -> ApiError {
    Problem::invalid(FieldError::new(field, ErrorCode::InvalidParameter)).into()
}

impl SearchQuery {
    fn search(&self) -> Result<Search, ApiError> {
        let sort = match self.sort.as_deref().unwrap_or("name") {
            "name" => SortKey::Name,
            "state" => SortKey::State,
            "prefab" => SortKey::Prefab,
            "node" => SortKey::Node,
            "memory" => SortKey::Memory,
            "cpu" => SortKey::Cpu,
            "disk" => SortKey::Disk,
            "created" => SortKey::Created,
            _ => return Err(invalid("sort")),
        };
        let descending = match self.order.as_deref().unwrap_or("asc") {
            "asc" => false,
            "desc" => true,
            _ => return Err(invalid("order")),
        };
        let nodes = list(self.node.as_deref())
            .iter()
            .map(|id| id.parse::<Uuid>().map_err(|_| invalid("node")))
            .collect::<Result<_, _>>()?;
        Ok(Search {
            filter: Filter {
                q: self.q.clone(),
                prefabs: list(self.prefab.as_deref()),
                nodes,
                states: list(self.state.as_deref()),
                upgrade: self.upgrade,
                suspended: self.suspended,
            },
            sort,
            descending,
            page: self.page.unwrap_or(1).max(1),
            per_page: self.per_page.unwrap_or(25),
            all_ids: self.all_ids,
        })
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FacetValueDto {
    pub value: String,
    /// Libellé : nom du nœud, ou dernière version du prefab (pour son icône).
    pub label: String,
    pub count: usize,
}

/// Nombre de serveurs par valeur, les autres filtres appliqués.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerFacetsDto {
    pub prefab: Vec<FacetValueDto>,
    pub node: Vec<FacetValueDto>,
    pub state: Vec<FacetValueDto>,
    /// Serveurs dont le prefab a une version plus récente.
    pub upgrade: usize,
    pub suspended: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServerPageDto {
    pub items: Vec<ServerDto>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
    pub facets: ServerFacetsDto,
    /// Dernière version disponible, pour les serveurs de la page qui ne l'ont pas.
    pub upgrades: BTreeMap<Uuid, String>,
    /// Tous les résultats (si `all_ids`), au plus 1000.
    pub ids: Option<Vec<Uuid>>,
}

fn facet_dto(values: Vec<FacetValue>) -> Vec<FacetValueDto> {
    values
        .into_iter()
        .map(|v| FacetValueDto {
            value: v.value,
            label: v.label,
            count: v.count,
        })
        .collect()
}

/// Liste paginée : recherche, filtres, tri et facettes. Permission : `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/servers/search", tag = "admin-servers", security(("bearer" = [])),
    params(SearchQuery),
    responses((status = 200, body = ServerPageDto), (status = 403, body = Problem), (status = 422, body = Problem)))]
pub async fn search(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Query(query): Query<SearchQuery>,
) -> Result<Json<ServerPageDto>, ApiError> {
    session.require(&iam, Permission::ServersRead).await?;
    let request = query.search()?;
    let page = search::search(&state, &request).await?;
    Ok(Json(ServerPageDto {
        items: page
            .items
            .into_iter()
            .map(|view| dto(&state, view))
            .collect(),
        total: page.total,
        page: request.page,
        per_page: request.per_page.clamp(1, search::MAX_PER_PAGE),
        facets: ServerFacetsDto {
            prefab: facet_dto(page.facets.prefab),
            node: facet_dto(page.facets.node),
            state: facet_dto(page.facets.state),
            upgrade: page.facets.upgrade,
            suspended: page.facets.suspended,
        },
        upgrades: page.latest.into_iter().collect(),
        ids: page.ids,
    }))
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BulkActionDto {
    Start,
    Stop,
    Restart,
    Kill,
    /// Vers la dernière version de leur prefab.
    Upgrade,
    Backup,
    Suspend,
    Unsuspend,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BulkRequest {
    pub action: BulkActionDto,
    /// 1 à 1000 serveurs.
    pub servers: Vec<Uuid>,
    /// Mise à jour : sauvegarde attendue d'abord.
    #[serde(default)]
    pub backup: bool,
}

/// Issue pour un serveur ; `error` : code stable de la raison du refus.
#[derive(Debug, Serialize, ToSchema)]
pub struct BulkOutcomeDto {
    pub server_id: Uuid,
    pub ok: bool,
    pub error: Option<String>,
}

/// Action sur plusieurs serveurs, chacun par son cas d'usage et son audit habituels ; un refus
/// n'arrête pas les autres. Permission de l'action : `servers.power`, `servers.upgrade` (et
/// `servers.backups.create` avec `backup`), `servers.backups.create`, `servers.suspend`.
#[utoipa::path(post, path = "/api/admin/v1/servers/bulk", tag = "admin-servers", security(("bearer" = [])),
    request_body = BulkRequest,
    responses((status = 200, body = [BulkOutcomeDto]), (status = 403, body = Problem), (status = 422, body = Problem)))]
pub async fn bulk_action(
    State(iam): State<IamState>,
    State(state): State<ServersState>,
    session: AdminSession,
    Json(request): Json<BulkRequest>,
) -> Result<(StatusCode, Json<Vec<BulkOutcomeDto>>), ApiError> {
    let (permission, action) = match request.action {
        BulkActionDto::Start => (
            Permission::ServersPower,
            BulkAction::Power(PowerAction::Start),
        ),
        BulkActionDto::Stop => (
            Permission::ServersPower,
            BulkAction::Power(PowerAction::Stop),
        ),
        BulkActionDto::Restart => (
            Permission::ServersPower,
            BulkAction::Power(PowerAction::Restart),
        ),
        BulkActionDto::Kill => (
            Permission::ServersPower,
            BulkAction::Power(PowerAction::Kill),
        ),
        BulkActionDto::Upgrade => (
            Permission::ServersUpgrade,
            BulkAction::Upgrade {
                backup: request.backup,
            },
        ),
        BulkActionDto::Backup => (Permission::ServersBackupsCreate, BulkAction::Backup),
        BulkActionDto::Suspend => (Permission::ServersSuspend, BulkAction::Suspend),
        BulkActionDto::Unsuspend => (Permission::ServersSuspend, BulkAction::Unsuspend),
    };
    // La permission d'abord (ADR-0007) : un refus est journalisé, quel que soit le corps.
    session.require(&iam, permission).await?;
    if matches!(action, BulkAction::Upgrade { backup: true }) {
        session
            .require(&iam, Permission::ServersBackupsCreate)
            .await?;
    }
    if request.servers.is_empty() || request.servers.len() > MAX_SERVERS {
        return Err(Problem::invalid(
            FieldError::new("servers", ErrorCode::ValueOutOfRange)
                .param("min", 1)
                .param("max", MAX_SERVERS),
        )
        .into());
    }
    // Doublons retirés, ordre de la sélection gardé.
    let mut seen = std::collections::HashSet::new();
    let servers: Vec<Uuid> = request
        .servers
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();
    let outcomes = bulk::run(&state, &session.context, action, &servers).await;
    Ok((
        StatusCode::OK,
        Json(
            outcomes
                .into_iter()
                .map(|o| BulkOutcomeDto {
                    server_id: o.server_id,
                    ok: o.error.is_none(),
                    error: o.error,
                })
                .collect(),
        ),
    ))
}
