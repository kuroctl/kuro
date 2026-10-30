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
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};

use crate::error_code::ErrorCode;
use crate::http::error::{ApiError, Problem, error_response, internal_error};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::prefabs::application::{self as prefabs, PrefabsDir, PrefabsError};
use crate::prefabs::infra::PrefabRecord;

#[derive(OpenApi)]
#[openapi(
    paths(list, get, locale, asset, import, delete, servers, reload, optimize_assets),
    tags((name = "admin-prefabs", description = "Prefabs : modèles de serveurs (ADR-0011)"))
)]
pub struct PrefabsApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    PgPool: FromRef<S>,
    PrefabsDir: FromRef<S>,
{
    Router::new()
        .route("/prefabs", route_get(list))
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
    /// `official` (dossier `prefabs/`, réimporté au démarrage) ou `uploaded` (panel).
    pub source: PrefabSourceDto,
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
            _ => PrefabSourceDto::Official,
        },
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

/// Icône (`icon`) ou bannière (`cover`) d'une version : PNG, JPEG ou WebP. Permission :
/// `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/prefabs/{id}/{version}/assets/{kind}", tag = "admin-prefabs",
    security(("bearer" = [])),
    params(("id" = String, Path), ("version" = String, Path), ("kind" = String, Path, description = "`icon` ou `cover`")),
    responses(
        (status = 200, description = "Image PNG, JPEG ou WebP", content_type = "image/*"),
        (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn asset(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, version, kind)): Path<(String, String, String)>,
    request: HeaderMap,
) -> Result<Response, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
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
    State(PrefabsDir(dir)): State<PrefabsDir>,
    session: AdminSession,
) -> Result<Json<ReloadReportDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let report = prefabs::reload(&pool, &session.context, &dir).await?;
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
