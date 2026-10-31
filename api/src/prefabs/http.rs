//! `/prefabs` : catalogue des modèles de serveurs. Permission : `prefabs.read`.

use std::collections::{BTreeMap, HashMap};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, FromRef, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get as route_get, post as route_post};
use kuro_prefab::Manifest;
use kuro_prefab::assets::AssetKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::{IntoParams, OpenApi, ToSchema};

use crate::error_code::ErrorCode;
use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::http::extract::{Json, Path, Query};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::prefabs::application::{self as prefabs, PrefabsDir, PrefabsError};
use crate::prefabs::infra::PrefabRecord;
use crate::prefabs::store::{StoreError, StoresState};

#[derive(OpenApi)]
#[openapi(
    paths(list, search, get, locale, asset, export, import, delete, servers, reload, optimize_assets, stores, store_install),
    tags((name = "admin-prefabs", description = "Prefabs : modèles de serveurs (ADR-0011)"))
)]
pub struct PrefabsApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    PgPool: FromRef<S>,
    PrefabsDir: FromRef<S>,
    StoresState: FromRef<S>,
{
    Router::new()
        .route("/prefabs", route_get(list))
        .route("/prefabs/search", route_get(search))
        .route("/prefabs/stores", route_get(stores))
        .route("/prefabs/stores/{store}/install", route_post(store_install))
        .route("/prefabs/reload", route_post(reload))
        // Archive bornée par `files::MAX_ARCHIVE_BYTES` : la limite par défaut (2 Mio) est levée
        // pour cette seule route.
        .route(
            "/prefabs/import",
            route_post(import).layer(DefaultBodyLimit::max(
                kuro_prefab::files::MAX_ARCHIVE_BYTES + 1024,
            )),
        )
        .route("/prefabs/{id}/{version}", route_get(get).delete(delete))
        .route("/prefabs/{id}/{version}/servers", route_get(servers))
        .route("/prefabs/{id}/{version}/locales/{lang}", route_get(locale))
        .route("/prefabs/{id}/{version}/assets/{kind}", route_get(asset))
        .route("/prefabs/{id}/{version}/export", route_get(export))
        .route(
            "/prefabs/{id}/{version}/assets/optimize",
            route_post(optimize_assets),
        )
}

impl IntoResponse for PrefabsError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => error_response(StatusCode::NOT_FOUND, ErrorCode::NotFound),
            Self::Invalid { reason, issues } => {
                Problem::new(StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::InvalidPrefab)
                    .param("reason", reason)
                    .param(
                        "issues",
                        issues
                            .iter()
                            .map(|issue| json!({ "path": issue.path, "message": issue.message }))
                            .collect::<Vec<_>>(),
                    )
                    .into_response()
            }
            Self::Exists => error_response(StatusCode::CONFLICT, ErrorCode::PrefabExists),
            Self::InUse => error_response(StatusCode::CONFLICT, ErrorCode::PrefabInUse),
            other => internal_error(&other, "prefabs request failed"),
        }
    }
}

impl From<PrefabsError> for ApiError {
    fn from(error: PrefabsError) -> Self {
        Self::from(error.into_response())
    }
}

impl IntoResponse for StoreError {
    fn into_response(self) -> Response {
        match self {
            Self::UnknownStore | Self::NotInIndex => {
                error_response(StatusCode::NOT_FOUND, ErrorCode::NotFound)
            }
            Self::Unavailable(reason) => {
                Problem::new(StatusCode::BAD_GATEWAY, ErrorCode::StoreUnavailable)
                    .param("reason", reason)
                    .into_response()
            }
            Self::Untrusted(reason) => {
                Problem::new(StatusCode::BAD_GATEWAY, ErrorCode::StoreUntrusted)
                    .param("reason", reason)
                    .into_response()
            }
            Self::Corrupted => error_response(StatusCode::BAD_GATEWAY, ErrorCode::StoreCorrupted),
            Self::Prefabs(error) => error.into_response(),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        Self::from(error.into_response())
    }
}

/// Un store de prefabs et ses versions ; `error` quand son index ne peut pas être lu ou cru.
#[derive(Debug, Serialize, ToSchema)]
pub struct StoreDto {
    pub name: String,
    pub url: String,
    /// Store injoignable, index absent ou signature refusée : rien n'en est proposé.
    pub error: Option<String>,
    pub prefabs: Vec<StorePrefabDto>,
}

/// Une version publiée par un store.
#[derive(Debug, Serialize, ToSchema)]
pub struct StorePrefabDto {
    pub id: String,
    pub version: String,
    pub category: String,
    pub tags: Vec<String>,
    pub names: BTreeMap<String, String>,
    pub descriptions: BTreeMap<String, String>,
    /// Versions de Kuro acceptées (`requires.kuro`), si déclaré.
    pub requires_kuro: Option<String>,
    /// Brouillon (`x-status`) : ce qui reste à vérifier.
    pub draft_reason: Option<String>,
    /// Images embarquées : `icon`, `cover`, `thumbnail`.
    pub assets: Vec<String>,
    /// Taille de l'archive en octets.
    pub size: u64,
    /// Cette API satisfait `requires.kuro`.
    pub compatible: bool,
    /// Déjà au catalogue.
    pub installed: bool,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct StoresQuery {
    /// Relire les index sans attendre l'expiration du cache (dix minutes).
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StoreInstallRequest {
    pub id: String,
    pub version: String,
}

/// Stores de prefabs configurés (ADR-0042) et les versions qu'ils publient, avec leur
/// compatibilité et leur présence au catalogue. Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/stores", tag = "admin-prefabs", security(("bearer" = [])),
    params(StoresQuery),
    responses((status = 200, body = [StoreDto]), (status = 403, body = Problem)))]
pub async fn stores(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(stores): State<StoresState>,
    session: AdminSession,
    Query(query): Query<StoresQuery>,
) -> Result<Json<Vec<StoreDto>>, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    let views = stores.catalog(&pool, query.refresh).await?;
    Ok(Json(
        views
            .into_iter()
            .map(|view| StoreDto {
                name: view.name,
                url: view.url,
                error: view.error,
                prefabs: view
                    .prefabs
                    .into_iter()
                    .map(|item| StorePrefabDto {
                        id: item.entry.id,
                        version: item.entry.version,
                        category: item.entry.category,
                        tags: item.entry.tags,
                        names: item.entry.names,
                        descriptions: item.entry.descriptions,
                        requires_kuro: item.entry.requires_kuro,
                        draft_reason: item.entry.draft,
                        assets: item.entry.assets,
                        size: item.entry.size,
                        compatible: item.compatible,
                        installed: item.installed,
                    })
                    .collect(),
            })
            .collect(),
    ))
}

/// Installe une version depuis un store : archive téléchargée à l'adresse de l'index signé,
/// somme SHA-256 vérifiée, puis importée comme une archive (source `store`). Permission :
/// `prefabs.manage`.
#[utoipa::path(post, path = "/api/admin/v1/prefabs/stores/{store}/install", tag = "admin-prefabs",
    security(("bearer" = [])), params(("store" = String, Path)), request_body = StoreInstallRequest,
    responses((status = 201, body = PrefabDto), (status = 403, body = Problem),
        (status = 404, description = "Store inconnu ou version absente de l'index", body = Problem),
        (status = 409, description = "Version déjà importée", body = Problem),
        (status = 422, description = "Prefab invalide", body = Problem),
        (status = 502, description = "Store injoignable, index non signé ou archive altérée", body = Problem)))]
pub async fn store_install(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(stores): State<StoresState>,
    session: AdminSession,
    Path(store): Path<String>,
    Json(request): Json<StoreInstallRequest>,
) -> Result<(StatusCode, Json<PrefabDto>), ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let imported = stores
        .install(
            &pool,
            &session.context,
            &store,
            &request.id,
            &request.version,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(detail(&pool, &imported.id, &imported.version).await?),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrefabSummaryDto {
    #[schema(example = "minecraft-java")]
    pub id: String,
    #[schema(example = "1.0.0")]
    pub version: String,
    #[schema(example = "game")]
    pub category: String,
    pub tags: Vec<String>,
    /// Langue → nom (`prefab.name` des traductions du prefab).
    pub names: BTreeMap<String, String>,
    /// Langue → description.
    pub descriptions: BTreeMap<String, String>,
    /// Déployable quelque part : faux si un minimum dépasse un plafond de la plateforme, ou si le
    /// compte Steam qu'il exige manque. Un brouillon reste déployable, sur un nœud de test.
    pub deployable: bool,
    /// Brouillon (`x-status: "draft: …"`) : déployable seulement sur un nœud de test (ADR-0011).
    pub test_nodes_only: bool,
    /// Ressources déclarées (`memory_mb`, `cpu_percent`, `disk_mb`, ADR-0031) : `min` bloquant,
    /// `recommended` pré-rempli à la création.
    pub resources: BTreeMap<String, ResourceNeedDto>,
    /// Ressources dont le minimum dépasse le plafond de la plateforme : non déployable ici.
    pub over_ceiling: Vec<String>,
    /// Version en brouillon (`x-status: "draft: …"`) : ce qui manque, tel que l'auteur l'a écrit.
    pub draft_reason: Option<String>,
    /// Le prefab exige le compte Steam de l'installation, qui n'est pas réglé (ADR-0033) : non
    /// déployable tant qu'il manque.
    pub steam_account_missing: bool,
    /// Images disponibles sur `/assets/{kind}` : `icon`, `cover`.
    #[schema(example = json!(["icon"]))]
    pub assets: Vec<String>,
    /// `official` (dossier `prefabs/`, réimporté au démarrage), `uploaded` (panel) ou `store`.
    pub source: PrefabSourceDto,
    /// Store d'origine d'une version `store` (ADR-0042).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    /// Serveurs déployés depuis cette version.
    pub servers: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub imported_at: OffsetDateTime,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResourceNeedDto {
    pub min: Option<u32>,
    pub recommended: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrefabDto {
    #[serde(flatten)]
    pub summary: PrefabSummaryDto,
    /// Manifest (format v1), dans l'ordre du fichier : ordre d'affichage des groupes et champs.
    #[schema(value_type = Object)]
    pub manifest: Manifest,
    /// Langues disponibles pour `/locales/{lang}`.
    pub locales: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrefabSourceDto {
    Official,
    Uploaded,
    /// Version d'essai d'un brouillon du Studio, hors catalogue.
    Trial,
    /// Installée depuis un store de prefabs (ADR-0042).
    Store,
}

/// Images et serveurs de chaque version, et plafonds de la plateforme, pour les résumés.
struct Counts {
    assets: HashMap<(String, String), Vec<String>>,
    servers: HashMap<(String, String), i64>,
    ceilings: [(&'static str, Option<i32>); 3],
    /// Un compte Steam d'installation est réglé (ADR-0033).
    steam_account: bool,
}

async fn counts(pool: &PgPool) -> Result<Counts, PrefabsError> {
    let platform = {
        let mut conn = pool.acquire().await?;
        crate::settings::infra::SettingsRepository::get(&mut conn).await?
    };
    Ok(Counts {
        assets: prefabs::asset_kinds(pool).await?,
        servers: prefabs::server_counts(pool).await?,
        ceilings: [
            ("memory_mb", platform.max_memory_mb),
            ("cpu_percent", platform.max_cpu_percent),
            ("disk_mb", platform.max_disk_mb),
        ],
        steam_account: crate::servers::application::steam_account::configured(pool).await?,
    })
}

fn summary(record: &PrefabRecord, manifest: &Manifest, counts: &Counts) -> PrefabSummaryDto {
    let key = (record.id.clone(), record.version.clone());
    let by_lang = |key: &str| -> BTreeMap<String, String> {
        record
            .locales
            .as_object()
            .map(|locales| {
                locales
                    .iter()
                    .filter_map(|(lang, messages)| {
                        let text = messages.get("prefab")?.get(key)?.as_str()?;
                        Some((lang.clone(), text.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let needs = &manifest.requires.resources;
    let over_ceiling: Vec<String> = counts
        .ceilings
        .iter()
        .filter(|(field, max)| {
            matches!((needs.min(field), max), (Some(min), Some(max)) if i64::from(min) > i64::from(*max))
        })
        .map(|(field, _)| (*field).to_owned())
        .collect();
    let steam_account_missing = manifest.needs_steam_account() && !counts.steam_account;
    PrefabSummaryDto {
        id: record.id.clone(),
        version: record.version.clone(),
        category: serde_json::to_value(manifest.category)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        tags: manifest.tags.clone(),
        names: by_lang("name"),
        descriptions: by_lang("description"),
        deployable: over_ceiling.is_empty() && !steam_account_missing,
        test_nodes_only: prefabs::is_draft(manifest),
        resources: needs
            .iter()
            .map(|(field, need)| {
                (
                    field.to_owned(),
                    ResourceNeedDto {
                        min: need.min,
                        recommended: need.recommended,
                    },
                )
            })
            .collect(),
        over_ceiling,
        draft_reason: prefabs::draft_reason(manifest),
        steam_account_missing,
        assets: counts.assets.get(&key).cloned().unwrap_or_default(),
        source: match record.source.as_str() {
            "uploaded" => PrefabSourceDto::Uploaded,
            "trial" => PrefabSourceDto::Trial,
            "store" => PrefabSourceDto::Store,
            _ => PrefabSourceDto::Official,
        },
        store: record.store.clone(),
        servers: counts.servers.get(&key).copied().unwrap_or(0),
        imported_at: record.imported_at,
    }
}

/// Prefabs importés, toutes versions. Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs", tag = "admin-prefabs", security(("bearer" = [])),
    responses((status = 200, body = [PrefabSummaryDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
) -> Result<Json<Vec<PrefabSummaryDto>>, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    let counts = counts(&pool).await?;
    let mut out = Vec::new();
    for record in prefabs::list(&pool).await? {
        let package = prefabs::to_package(record.clone())?;
        out.push(summary(&record, &package.manifest, &counts));
    }
    Ok(Json(out))
}

/// Prefabs par page, au plus.
const MAX_SEARCH_PER_PAGE: usize = 50;

/// Critères de la recherche dans le catalogue.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PrefabSearchQuery {
    /// Identifiant, nom (toutes langues) ou étiquette ; sans casse.
    pub q: Option<String>,
    /// À partir de 1.
    pub page: Option<usize>,
    /// 1 à 50, 20 par défaut.
    pub per_page: Option<usize>,
}

/// Un prefab et toutes ses versions.
#[derive(Debug, Serialize, ToSchema)]
pub struct PrefabGroupDto {
    #[schema(example = "minecraft-java")]
    pub id: String,
    /// Langue → nom, d'après la dernière version.
    pub names: BTreeMap<String, String>,
    /// Versions importées, de la plus récente à la plus ancienne (semver).
    #[schema(example = json!(["1.4.0", "1.3.0"]))]
    pub versions: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrefabGroupPageDto {
    pub items: Vec<PrefabGroupDto>,
    /// Prefabs trouvés, toutes pages.
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
}

/// Nom de chaque langue, d'après `prefab.name` des traductions.
fn names_of(record: &PrefabRecord) -> BTreeMap<String, String> {
    record
        .locales
        .as_object()
        .map(|locales| {
            locales
                .iter()
                .filter_map(|(lang, messages)| {
                    let name = messages.get("prefab")?.get("name")?.as_str()?;
                    Some((lang.clone(), name.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Regroupe les versions par prefab (versions décroissantes), filtre sur `q` et trie par id.
fn groups(records: &[PrefabRecord], q: &str) -> Vec<PrefabGroupDto> {
    let mut by_id: BTreeMap<&str, Vec<&PrefabRecord>> = BTreeMap::new();
    for record in records {
        by_id.entry(&record.id).or_default().push(record);
    }
    let needle = q.trim().to_lowercase();
    by_id
        .into_iter()
        .filter_map(|(id, mut versions)| {
            // Semver d'abord ; une version illisible passe après, dans l'ordre du texte.
            versions.sort_by(|a, b| {
                match (
                    a.version.parse::<semver::Version>(),
                    b.version.parse::<semver::Version>(),
                ) {
                    (Ok(a), Ok(b)) => b.cmp(&a),
                    (Ok(_), Err(_)) => std::cmp::Ordering::Less,
                    (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                    (Err(_), Err(_)) => b.version.cmp(&a.version),
                }
            });
            let latest = versions.first()?;
            let names = names_of(latest);
            let tags = serde_json::from_str::<Value>(&latest.manifest)
                .ok()
                .and_then(|manifest| manifest.get("tags").and_then(Value::as_array).cloned())
                .unwrap_or_default();
            let found = needle.is_empty()
                || id.to_lowercase().contains(&needle)
                || names
                    .values()
                    .any(|name| name.to_lowercase().contains(&needle))
                || tags
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|tag| tag.to_lowercase().contains(&needle));
            found.then(|| PrefabGroupDto {
                id: id.to_owned(),
                names,
                versions: versions
                    .iter()
                    .map(|record| record.version.clone())
                    .collect(),
            })
        })
        .collect()
}

/// Recherche paginée dans le catalogue, un résultat par prefab. Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/search", tag = "admin-prefabs", security(("bearer" = [])),
    params(PrefabSearchQuery),
    responses((status = 200, body = PrefabGroupPageDto), (status = 403, body = Problem)))]
pub async fn search(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Query(query): Query<PrefabSearchQuery>,
) -> Result<Json<PrefabGroupPageDto>, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    let found = groups(
        &prefabs::list(&pool).await?,
        query.q.as_deref().unwrap_or_default(),
    );
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, MAX_SEARCH_PER_PAGE);
    let total = found.len();
    let items = found
        .into_iter()
        .skip(page.saturating_sub(1).saturating_mul(per_page))
        .take(per_page)
        .collect();
    Ok(Json(PrefabGroupPageDto {
        items,
        total,
        page,
        per_page,
    }))
}

/// Manifest d'une version. Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}", tag = "admin-prefabs", security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path)),
    responses((status = 200, body = PrefabDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, version)): Path<(String, String)>,
) -> Result<Json<PrefabDto>, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    Ok(Json(detail(&pool, &id, &version).await?))
}

pub(crate) async fn detail(
    pool: &PgPool,
    id: &str,
    version: &str,
) -> Result<PrefabDto, PrefabsError> {
    let record = prefabs::get(pool, id, version).await?;
    let package = prefabs::to_package(record.clone())?;
    let counts = counts(pool).await?;
    Ok(PrefabDto {
        summary: summary(&record, &package.manifest, &counts),
        locales: package.locales.keys().cloned().collect(),
        manifest: package.manifest,
    })
}

/// Traductions d'une langue (libellés, aides, options). Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}/locales/{lang}", tag = "admin-prefabs",
    security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path), ("lang" = String, Path)),
    responses((status = 200, body = Object), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn locale(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, version, lang)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    let record = prefabs::get(&pool, &id, &version).await?;
    let messages = record
        .locales
        .get(&lang)
        .cloned()
        .ok_or(PrefabsError::NotFound)?;
    Ok(Json(messages))
}

/// Icône (`icon`) ou bannière (`cover`) d'une version : PNG, JPEG ou WebP. Toute session admin :
/// un serveur s'affiche avec les images de son prefab même sans droit sur les prefabs.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}/assets/{kind}", tag = "admin-prefabs",
    security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path), ("kind" = String, Path, description = "`icon` ou `cover`")),
    responses(
        (status = 200, description = "Image PNG, JPEG ou WebP", content_type = "image/*"),
        (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn asset(
    State(pool): State<PgPool>,
    _session: AdminSession,
    Path((id, version, kind)): Path<(String, String, String)>,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    let kind = AssetKind::parse(&kind).ok_or(PrefabsError::NotFound)?;
    let (content_type, data) = prefabs::asset(&pool, &id, &version, kind).await?;
    // Une image peut être optimisée après coup : revalidée à chaque affichage (`ETag`, 304 si
    // inchangée). Ouverte directement, elle ne peut rien exécuter (`sandbox`), et le navigateur
    // ne devine pas un autre type.
    let digest = Sha256::digest(&data);
    let etag = format!(
        "\"{}\"",
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    if request
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == etag.as_bytes())
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }
    let headers = [
        (header::CONTENT_TYPE, content_type),
        (header::CACHE_CONTROL, "private, no-cache".to_owned()),
        (header::ETAG, etag),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox".to_owned(),
        ),
    ];
    Ok((headers, data).into_response())
}

/// Archive `.kprefab` d'une version (ADR-0042) : un zip avec un dossier de tête `<id>/`, prêt
/// à être importé ailleurs ou déposé dans le dépôt des prefabs. Une version officielle dont le
/// dossier est présent garde ses fichiers d'origine ; une version importée a son manifest
/// régénéré (sans commentaires). Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}/export", tag = "admin-prefabs",
    security(("bearer" = [])), params(("id" = String, Path), ("version" = String, Path)),
    responses(
        (status = 200, description = "Archive zip `<id>-<version>.kprefab`", content_type = "application/zip"),
        (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn export(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(prefabs_dir): State<PrefabsDir>,
    session: AdminSession,
    Path((id, version)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    let (name, bytes) =
        crate::prefabs::export::version_archive(&pool, &prefabs_dir.dir, &id, &version).await?;
    Ok(archive_response(&name, bytes))
}

/// Réponse d'une archive à télécharger : zip, nom de fichier, jamais mise en cache.
pub fn archive_response(name: &str, bytes: Vec<u8>) -> Response {
    let headers = [
        (header::CONTENT_TYPE, "application/zip".to_owned()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        ),
        (header::CACHE_CONTROL, "private, no-store".to_owned()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
    ];
    (headers, bytes).into_response()
}

/// Importe une archive `.kprefab` (zip, 10 Mio au plus) envoyée telle quelle dans le corps. Même
/// validation que les prefabs officiels ; une version déjà présente est refusée (immuable).
/// Permission : `prefabs.manage`.
#[utoipa::path(post, path = "/api/admin/v1/prefabs/import", tag = "admin-prefabs", security(("bearer" = [])),
    request_body(content = Vec<u8>, content_type = "application/zip"),
    responses((status = 201, body = PrefabDto), (status = 403, body = Problem),
        (status = 409, description = "Version déjà importée", body = Problem),
        (status = 413, description = "Archive trop grosse"),
        (status = 422, description = "Archive ou prefab invalide (`params.reason`, `params.issues`)", body = Problem)))]
pub async fn import(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    body: Bytes,
) -> Result<(StatusCode, Json<PrefabDto>), ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let imported = prefabs::import_archive(&pool, &session.context, &body).await?;
    Ok((
        StatusCode::CREATED,
        Json(detail(&pool, &imported.id, &imported.version).await?),
    ))
}

/// Supprime une version qu'aucun serveur n'utilise. Un prefab officiel revient au prochain
/// démarrage de l'API. Permission : `prefabs.manage`.
#[utoipa::path(delete, path = "/api/admin/v1/prefabs/{id}/{version}", tag = "admin-prefabs", security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Des serveurs utilisent cette version", body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, version)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    prefabs::delete(&pool, &session.context, &id, &version).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrefabServerDto {
    pub id: uuid::Uuid,
    pub name: String,
    #[schema(example = "running")]
    pub state: String,
}

/// Serveurs déployés depuis cette version. Permissions : `prefabs.read` et `servers.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}/servers", tag = "admin-prefabs", security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path)),
    responses((status = 200, body = [PrefabServerDto]), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn servers(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, version)): Path<(String, String)>,
) -> Result<Json<Vec<PrefabServerDto>>, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    session.require(&iam, Permission::ServersRead).await?;
    let servers = prefabs::servers_of(&pool, &id, &version).await?;
    Ok(Json(
        servers
            .into_iter()
            .map(|(id, name, state)| PrefabServerDto { id, name, state })
            .collect(),
    ))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InvalidPrefabDto {
    /// Dossier du prefab refusé.
    pub folder: String,
    pub reason: String,
}

/// Résultat d'un rechargement du dossier des prefabs officiels.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReloadReportDto {
    /// Nouvelles versions (`id@version`).
    pub imported: Vec<String>,
    pub unchanged: Vec<String>,
    /// Images ajoutées à une version déjà présente (`id@version: icon, cover`).
    pub assets_added: Vec<String>,
    /// Modifiées sans changement de version : la version stockée est gardée.
    pub conflicts: Vec<String>,
    pub invalid: Vec<InvalidPrefabDto>,
}

/// Dimensions et poids d'une image.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImageMeasureDto {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

impl From<kuro_prefab::media::Measure> for ImageMeasureDto {
    fn from(measure: kuro_prefab::media::Measure) -> Self {
        Self {
            width: measure.width,
            height: measure.height,
            bytes: measure.bytes,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OptimizedAssetDto {
    /// `icon`, `cover` ou `thumbnail`.
    pub kind: String,
    pub before: ImageMeasureDto,
    pub after: ImageMeasureDto,
    /// Plus petite que la convention : jamais agrandie, à remplacer par une image plus grande.
    pub small: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FailedAssetDto {
    pub kind: String,
    pub reason: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OptimizeReportDto {
    pub optimized: Vec<OptimizedAssetDto>,
    /// Déjà conformes et optimisées.
    pub unchanged: Vec<String>,
    /// Laissées telles quelles (illisibles, ou trop lourdes une fois traitées).
    pub failed: Vec<FailedAssetDto>,
}

/// Met les images de la version aux dimensions convenues (icône 64×64, bannière 1600×600,
/// vignette 600×800 ; recadrées au centre, jamais agrandies) et les compresse sans perte.
/// Permission : `prefabs.manage`.
#[utoipa::path(post, path = "/api/admin/v1/prefabs/{id}/{version}/assets/optimize", tag = "admin-prefabs",
    security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path)),
    responses((status = 200, body = OptimizeReportDto), (status = 403, body = Problem),
        (status = 404, body = Problem)))]
pub async fn optimize_assets(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, version)): Path<(String, String)>,
) -> Result<Json<OptimizeReportDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let report = prefabs::optimize_assets(&pool, &session.context, &id, &version).await?;
    Ok(Json(OptimizeReportDto {
        optimized: report
            .optimized
            .into_iter()
            .map(|asset| OptimizedAssetDto {
                kind: asset.kind.as_str().to_owned(),
                before: asset.before.into(),
                after: asset.after.into(),
                small: asset.small,
            })
            .collect(),
        unchanged: report
            .unchanged
            .into_iter()
            .map(|kind| kind.as_str().to_owned())
            .collect(),
        failed: report
            .failed
            .into_iter()
            .map(|(kind, reason)| FailedAssetDto {
                kind: kind.as_str().to_owned(),
                reason,
            })
            .collect(),
    }))
}

/// Relit le dossier des prefabs officiels sans redémarrer l'API : nouvelles versions et images
/// manquantes importées. Permission : `prefabs.manage`.
#[utoipa::path(post, path = "/api/admin/v1/prefabs/reload", tag = "admin-prefabs", security(("bearer" = [])),
    responses((status = 200, body = ReloadReportDto), (status = 403, body = Problem)))]
pub async fn reload(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(prefabs_dir): State<PrefabsDir>,
    session: AdminSession,
) -> Result<Json<ReloadReportDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let report = prefabs::reload(&pool, &session.context, &prefabs_dir.dir).await?;
    Ok(Json(ReloadReportDto {
        imported: report.imported,
        unchanged: report.unchanged,
        assets_added: report.assets_added,
        conflicts: report.conflicts,
        invalid: report
            .invalid
            .into_iter()
            .map(|(folder, reason)| InvalidPrefabDto { folder, reason })
            .collect(),
    }))
}
