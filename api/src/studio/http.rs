//! `/studio` : Prefab Studio (brouillons de prefabs). Permission : `prefabs.manage`, sauf le
//! schéma du format (`prefabs.read`).

use std::collections::BTreeMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, FromRef, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get as route_get, post as route_post, put as route_put};
use kuro_prefab::assets::{AssetKind, MAX_ASSET_BYTES};
use kuro_prefab::compose::{Conversion, NoteKind};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::OffsetDateTime;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use crate::error_code::ErrorCode;
use crate::http::error::{ApiError, Problem, error_response};
use crate::http::extract::{Json, Path};
use crate::iam::application::IamState;
use crate::iam::domain::Permission;
use crate::iam::http::AdminSession;
use crate::prefabs::application::{PrefabsDir, PrefabsError};
use crate::prefabs::http::{PrefabDto, detail};
use crate::servers::application::ServersState;
use crate::servers::infra::ServerRepository;
use crate::studio::application::{self as studio, Check, DraftFiles, Start};
use crate::studio::infra::DraftRecord;
use crate::studio::trials::{self, EndReason};

/// Corps d'un enregistrement : fichiers texte bornés à 32 Mio une fois lus, plus l'échappement
/// JSON.
const MAX_DRAFT_BODY: usize = 40 * 1024 * 1024;

/// JSON Schema publié du format v1, pour l'autocomplétion de l'éditeur.
const SCHEMA: &str = include_str!("../../../shared/prefab/schema/prefab.v1.schema.json");

#[derive(OpenApi)]
#[openapi(
    paths(schema, list, create, get, save, delete, validate, compose, egg, put_asset, asset, delete_asset, import_steam_assets, publish, export, start_trial),
    tags((name = "admin-studio", description = "Prefab Studio : brouillons de prefabs (ADR-0011)"))
)]
pub struct StudioApi;

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    IamState: FromRef<S>,
    PgPool: FromRef<S>,
    PrefabsDir: FromRef<S>,
    ServersState: FromRef<S>,
{
    let draft_body = DefaultBodyLimit::max(MAX_DRAFT_BODY);
    Router::new()
        .route("/studio/schema", route_get(schema))
        .route("/studio/drafts", route_get(list).post(create))
        .route("/studio/validate", route_post(validate).layer(draft_body))
        .route(
            "/studio/drafts/{id}",
            route_get(get).put(save).delete(delete).layer(draft_body),
        )
        .route(
            "/studio/drafts/{id}/assets/{kind}",
            route_put(put_asset)
                .get(asset)
                .delete(delete_asset)
                .layer(DefaultBodyLimit::max(MAX_ASSET_BYTES as usize + 1024)),
        )
        .route(
            "/studio/drafts/{id}/assets/steam",
            route_post(import_steam_assets),
        )
        .route("/studio/drafts/{id}/publish", route_post(publish))
        .route("/studio/drafts/{id}/export", route_get(export))
        .route("/studio/drafts/{id}/trial", route_post(start_trial))
        .route(
            "/studio/compose",
            route_post(compose).layer(DefaultBodyLimit::max(
                kuro_prefab::compose::MAX_COMPOSE_BYTES * 2,
            )),
        )
        .route(
            "/studio/egg",
            route_post(egg).layer(DefaultBodyLimit::max(kuro_prefab::egg::MAX_EGG_BYTES * 2)),
        )
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IssueDto {
    /// Emplacement dans le manifest (`services.server.image`).
    pub path: String,
    pub message: String,
}

/// Validation des fichiers, comme à l'import.
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckDto {
    /// Importable tel quel.
    pub valid: bool,
    /// Refus global sans emplacement (YAML illisible, traduction invalide…).
    pub reason: Option<String>,
    pub issues: Vec<IssueDto>,
}

impl From<Check> for CheckDto {
    fn from(check: Check) -> Self {
        Self {
            valid: check.valid(),
            reason: check.reason,
            issues: check
                .issues
                .into_iter()
                .map(|issue| IssueDto {
                    path: issue.path,
                    message: issue.message,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DraftSummaryDto {
    pub id: Uuid,
    /// `id` et `version` lus du manifest au dernier enregistrement (vides s'il ne se lisait pas).
    pub prefab_id: Option<String>,
    pub prefab_version: Option<String>,
    /// Version dont le brouillon est parti.
    pub base_id: Option<String>,
    pub base_version: Option<String>,
    /// Images déposées : `icon`, `cover`, `thumbnail`.
    pub assets: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

impl From<&DraftRecord> for DraftSummaryDto {
    fn from(record: &DraftRecord) -> Self {
        Self {
            id: record.id,
            prefab_id: record.prefab_id.clone(),
            prefab_version: record.prefab_version.clone(),
            base_id: record.base_id.clone(),
            base_version: record.base_version.clone(),
            assets: record.assets.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DraftDto {
    #[serde(flatten)]
    pub summary: DraftSummaryDto,
    /// Chemin relatif → contenu texte (`prefab.yaml`, `locales/fr.json`, gabarits).
    pub files: BTreeMap<String, String>,
    pub check: CheckDto,
    /// Essai en cours sur un nœud de test.
    pub trial: Option<DraftTrialDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DraftTrialDto {
    pub server_id: Uuid,
    pub name: String,
    #[schema(example = "running")]
    pub state: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: OffsetDateTime,
}

async fn draft_dto(pool: &PgPool, record: &DraftRecord) -> Result<DraftDto, PrefabsError> {
    let files = studio::files_of(record);
    let check = studio::check(&files)?;
    let trial = {
        let mut conn = pool.acquire().await?;
        let mut trial = None;
        for id in ServerRepository::trials_of(&mut conn, record.id).await? {
            if let Some(server) = ServerRepository::get(&mut conn, id).await?
                && let Some(expires_at) = server.expires_at
            {
                trial = Some(DraftTrialDto {
                    server_id: server.id,
                    name: server.name,
                    state: server.state,
                    expires_at,
                });
            }
        }
        trial
    };
    Ok(DraftDto {
        summary: record.into(),
        files,
        check: check.into(),
        trial,
    })
}

/// JSON Schema du format v1 (`prefab.yaml`). Permission : `prefabs.read`.
#[utoipa::path(get, path = "/api/admin/v1/studio/schema", tag = "admin-studio", security(("bearer" = [])),
    responses((status = 200, body = Object), (status = 403, body = Problem)))]
pub async fn schema(
    State(iam): State<IamState>,
    session: AdminSession,
) -> Result<Response, ApiError> {
    session.require(&iam, Permission::PrefabsRead).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], SCHEMA).into_response())
}

/// Brouillons, le plus récemment modifié d'abord.
#[utoipa::path(get, path = "/api/admin/v1/studio/drafts", tag = "admin-studio", security(("bearer" = [])),
    responses((status = 200, body = [DraftSummaryDto]), (status = 403, body = Problem)))]
pub async fn list(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
) -> Result<Json<Vec<DraftSummaryDto>>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let drafts = studio::list(&pool).await?;
    Ok(Json(drafts.iter().map(DraftSummaryDto::from).collect()))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateDraftRequest {
    /// Version à copier (version suivante, images reprises).
    pub from: Option<PrefabVersionRef>,
    /// `docker-compose.yml` à convertir. Aucun des trois : prefab d'exemple.
    pub compose: Option<ComposeRequest>,
    /// Egg Pterodactyl à convertir (ADR-0027).
    pub egg: Option<EggRequest>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EggRequest {
    /// JSON de l'egg, `PTDL_v1` ou `PTDL_v2` (512 Kio au plus).
    pub text: String,
    /// Identifiant voulu ; sinon dérivé du nom de l'egg.
    pub id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ComposeRequest {
    /// Contenu du `docker-compose.yml` (256 Kio au plus).
    pub text: String,
    /// Identifiant voulu ; sinon `name:` du compose, ou son premier service.
    pub id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComposeNoteKindDto {
    /// Convertie avec une adaptation.
    Adapted,
    /// Sans effet dans Kuro.
    Ignored,
    /// Contraire au durcissement ou sans équivalent.
    Refused,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComposeNoteDto {
    /// Emplacement dans le compose (`services.db.privileged`).
    pub path: String,
    pub kind: ComposeNoteKindDto,
    pub message: String,
}

/// Aperçu de la conversion d'un `docker-compose.yml`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ComposeDto {
    pub id: String,
    pub files: BTreeMap<String, String>,
    pub notes: Vec<ComposeNoteDto>,
    /// Validation du prefab produit.
    pub check: CheckDto,
}

/// Convertit un egg Pterodactyl sans rien enregistrer (ADR-0027) : fichiers du futur brouillon,
/// ce qui a été adapté, ignoré ou refusé (emplacements dans l'egg), et la validation du résultat.
#[utoipa::path(post, path = "/api/admin/v1/studio/egg", tag = "admin-studio", security(("bearer" = [])),
    request_body = EggRequest,
    responses((status = 200, body = ComposeDto), (status = 403, body = Problem),
        (status = 422, description = "Pas un egg lisible (PTDL_v1/v2, image, démarrage)", body = Problem)))]
pub async fn egg(
    State(iam): State<IamState>,
    session: AdminSession,
    Json(request): Json<EggRequest>,
) -> Result<Json<ComposeDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let conversion = studio::convert_egg(&request.text, request.id.as_deref())?;
    conversion_dto(conversion)
}

/// Aperçu d'une conversion (compose ou egg) : fichiers, notes, validation.
fn conversion_dto(conversion: Conversion) -> Result<Json<ComposeDto>, ApiError> {
    let check = studio::check(&conversion.files)?;
    Ok(Json(ComposeDto {
        id: conversion.id,
        files: conversion.files,
        notes: conversion
            .notes
            .into_iter()
            .map(|note| ComposeNoteDto {
                path: note.path,
                kind: match note.kind {
                    NoteKind::Adapted => ComposeNoteKindDto::Adapted,
                    NoteKind::Ignored => ComposeNoteKindDto::Ignored,
                    NoteKind::Refused => ComposeNoteKindDto::Refused,
                },
                message: note.message,
            })
            .collect(),
        check: check.into(),
    }))
}

/// Convertit un `docker-compose.yml` sans rien enregistrer : fichiers du futur brouillon, ce qui a
/// été adapté, ignoré ou refusé, et la validation du résultat.
#[utoipa::path(post, path = "/api/admin/v1/studio/compose", tag = "admin-studio", security(("bearer" = [])),
    request_body = ComposeRequest,
    responses((status = 200, body = ComposeDto), (status = 403, body = Problem),
        (status = 422, description = "Pas un docker-compose lisible, aucun service", body = Problem)))]
pub async fn compose(
    State(iam): State<IamState>,
    session: AdminSession,
    Json(request): Json<ComposeRequest>,
) -> Result<Json<ComposeDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let conversion = studio::convert_compose(&request.text, request.id.as_deref())?;
    conversion_dto(conversion)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PrefabVersionRef {
    pub id: String,
    pub version: String,
}

/// Crée un brouillon : prefab d'exemple, copie d'une version, conversion d'un docker-compose ou d'un
/// egg Pterodactyl.
#[utoipa::path(post, path = "/api/admin/v1/studio/drafts", tag = "admin-studio", security(("bearer" = [])),
    request_body = CreateDraftRequest,
    responses((status = 201, body = DraftDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn create(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(prefabs_dir): State<PrefabsDir>,
    session: AdminSession,
    Json(request): Json<CreateDraftRequest>,
) -> Result<(StatusCode, Json<DraftDto>), ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let start = match (request.from, request.compose, request.egg) {
        (Some(from), _, _) => Start::From {
            id: from.id,
            version: from.version,
            assets: true,
            new_id: None,
        },
        (None, Some(compose), _) => Start::Compose {
            text: compose.text,
            id: compose.id,
        },
        (None, None, Some(egg)) => Start::Egg {
            text: egg.text,
            id: egg.id,
        },
        (None, None, None) => Start::Blank,
    };
    let id = studio::create(&pool, &session.context, &prefabs_dir.dir, start).await?;
    let record = studio::get(&pool, id).await?;
    Ok((StatusCode::CREATED, Json(draft_dto(&pool, &record).await?)))
}

#[utoipa::path(get, path = "/api/admin/v1/studio/drafts/{id}", tag = "admin-studio", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = DraftDto), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn get(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<DraftDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let record = studio::get(&pool, id).await?;
    Ok(Json(draft_dto(&pool, &record).await?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct DraftFilesRequest {
    /// Chemin relatif → contenu texte. `prefab.yaml` obligatoire ; 256 fichiers, 2 Mio chacun.
    pub files: DraftFiles,
}

/// Enregistre les fichiers du brouillon, même invalides, et rend leur validation.
#[utoipa::path(put, path = "/api/admin/v1/studio/drafts/{id}", tag = "admin-studio", security(("bearer" = [])),
    params(("id" = Uuid, Path)), request_body = DraftFilesRequest,
    responses((status = 200, body = DraftDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Chemin refusé, fichier trop gros, `prefab.yaml` absent", body = Problem)))]
pub async fn save(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<DraftFilesRequest>,
) -> Result<Json<DraftDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    studio::save(&pool, &session.context, id, &request.files).await?;
    let record = studio::get(&pool, id).await?;
    Ok(Json(draft_dto(&pool, &record).await?))
}

#[utoipa::path(delete, path = "/api/admin/v1/studio/drafts/{id}", tag = "admin-studio", security(("bearer" = [])),
    params(("id" = Uuid, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(servers): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    studio::get(&pool, id).await?;
    trials::end(&servers, &session.context, id, EndReason::DraftDeleted).await?;
    studio::delete(&pool, &session.context, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Valide des fichiers sans rien enregistrer (pendant la saisie).
#[utoipa::path(post, path = "/api/admin/v1/studio/validate", tag = "admin-studio", security(("bearer" = [])),
    request_body = DraftFilesRequest,
    responses((status = 200, body = CheckDto), (status = 403, body = Problem),
        (status = 422, description = "Chemin refusé, fichier trop gros, `prefab.yaml` absent", body = Problem)))]
pub async fn validate(
    State(iam): State<IamState>,
    session: AdminSession,
    Json(request): Json<DraftFilesRequest>,
) -> Result<Json<CheckDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    Ok(Json(studio::check(&request.files)?.into()))
}

/// Dépose ou remplace une image (`icon`, `cover`, `thumbnail`) : PNG, JPEG ou WebP, 2 Mio.
#[utoipa::path(put, path = "/api/admin/v1/studio/drafts/{id}/assets/{kind}", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("kind" = String, Path)),
    request_body(content = Vec<u8>, content_type = "image/*"),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 422, description = "Pas une image PNG, JPEG ou WebP", body = Problem)))]
pub async fn put_asset(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, kind)): Path<(Uuid, String)>,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let kind = AssetKind::parse(&kind).ok_or(PrefabsError::NotFound)?;
    studio::put_asset(&pool, &session.context, id, kind, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/admin/v1/studio/drafts/{id}/assets/{kind}", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("kind" = String, Path)),
    responses((status = 200, description = "Image PNG, JPEG ou WebP", content_type = "image/*"),
        (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn asset(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, kind)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let kind = AssetKind::parse(&kind).ok_or(PrefabsError::NotFound)?;
    let (content_type, data) = studio::asset(&pool, id, kind).await?;
    let headers = [
        (header::CONTENT_TYPE, content_type),
        (header::CACHE_CONTROL, "private, no-store".to_owned()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; sandbox".to_owned(),
        ),
    ];
    Ok((headers, data).into_response())
}

#[utoipa::path(delete, path = "/api/admin/v1/studio/drafts/{id}/assets/{kind}", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path), ("kind" = String, Path)),
    responses((status = 204), (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn delete_asset(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path((id, kind)): Path<(Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let kind = AssetKind::parse(&kind).ok_or(PrefabsError::NotFound)?;
    studio::delete_asset(&pool, &session.context, id, kind).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Images importées du magasin Steam.
#[derive(Debug, Serialize, ToSchema)]
pub struct SteamImportDto {
    /// Application Steam du jeu, lue dans le manifest enregistré (`steam.app`).
    #[schema(example = 730)]
    pub app: u32,
    /// Nom du jeu sur le magasin.
    #[schema(example = "Counter-Strike 2")]
    pub name: String,
    /// Images posées : `icon`, `cover` (bannière), `thumbnail` (jaquette).
    pub imported: Vec<String>,
    /// Source de chaque image posée : `steamgriddb` (si sa clé est réglée) ou `steam`.
    #[schema(example = json!({ "icon": "steamgriddb", "cover": "steam" }))]
    pub sources: BTreeMap<String, String>,
    /// Introuvables ou refusées (format, taille) : les images du brouillon restent.
    pub missing: Vec<String>,
}

impl IntoResponse for studio::SteamImportError {
    fn into_response(self) -> Response {
        match self {
            Self::Prefabs(error) => error.into_response(),
            Self::NoGame => error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::SteamGameMissing,
            ),
            Self::GameNotFound => error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::SteamGameNotFound,
            ),
            Self::Unavailable(reason) => {
                tracing::warn!(%reason, "steam store unavailable");
                error_response(StatusCode::BAD_GATEWAY, ErrorCode::ProviderUnavailable)
            }
        }
    }
}

/// Importe les images du jeu Steam déclaré par le manifest enregistré (`steam.app`) : icône de
/// la communauté, sinon logo (`icon`), bannière (`cover`), jaquette (`thumbnail`). Les images absentes du magasin sont
/// listées, celles du brouillon restent.
#[utoipa::path(post, path = "/api/admin/v1/studio/drafts/{id}/assets/steam", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 200, body = SteamImportDto), (status = 403, body = Problem),
        (status = 404, body = Problem),
        (status = 422, description = "Aucun jeu Steam déclaré, ou inconnu du magasin", body = Problem),
        (status = 502, description = "Steam injoignable", body = Problem)))]
pub async fn import_steam_assets(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(servers): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Json<SteamImportDto>, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let import = studio::import_steam_assets(&pool, &session.context, &servers.content, id)
        .await
        .map_err(|error| ApiError::from(error.into_response()))?;
    Ok(Json(SteamImportDto {
        app: import.app,
        name: import.name,
        imported: import
            .imported
            .iter()
            .map(|(kind, _)| kind.as_str().to_owned())
            .collect(),
        sources: import
            .imported
            .iter()
            .map(|(kind, origin)| (kind.as_str().to_owned(), origin.source().to_owned()))
            .collect(),
        missing: import
            .missing
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect(),
    }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StartTrialRequest {
    /// Nœud de test (`trials`) qui recevra le serveur d'essai.
    pub node_id: Uuid,
    /// Nom du serveur d'essai.
    #[schema(example = "Essai de my-prefab")]
    pub name: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TrialStartedDto {
    pub server_id: Uuid,
    /// Version d'essai déployée (`1.0.1-trial.<n>`), hors catalogue.
    pub version: String,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub expires_at: OffsetDateTime,
}

/// Déploie le brouillon tel qu'il est enregistré sur un nœud de test, pour deux heures : version
/// d'essai cachée, serveur ordinaire supprimé à son expiration. Remplace l'essai en cours.
/// Permissions : `prefabs.manage` et `servers.create`.
#[utoipa::path(post, path = "/api/admin/v1/studio/drafts/{id}/trial", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path)), request_body = StartTrialRequest,
    responses((status = 201, body = TrialStartedDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Nœud hors ligne ou pas de test, aucun port libre", body = Problem),
        (status = 422, description = "Brouillon invalide", body = Problem)))]
pub async fn start_trial(
    State(iam): State<IamState>,
    State(servers): State<ServersState>,
    session: AdminSession,
    Path(id): Path<Uuid>,
    Json(request): Json<StartTrialRequest>,
) -> Result<(StatusCode, Json<TrialStartedDto>), ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    session.require(&iam, Permission::ServersCreate).await?;
    let view = trials::start(
        &servers,
        &session.context,
        id,
        request.node_id,
        &request.name,
        OffsetDateTime::now_utc(),
    )
    .await?;
    let server = view.server;
    Ok((
        StatusCode::CREATED,
        Json(TrialStartedDto {
            server_id: server.id,
            version: server.prefab_version,
            expires_at: server.expires_at.unwrap_or_else(OffsetDateTime::now_utc),
        }),
    ))
}

/// Archive `.kprefab` du brouillon (ADR-0042) : ses fichiers enregistrés et ses images, prête à
/// être déposée dans le dépôt des prefabs ou importée ailleurs. Permission : `prefabs.manage`.
#[utoipa::path(get, path = "/api/admin/v1/studio/drafts/{id}/export", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses(
        (status = 200, description = "Archive zip `<id>-<version>.kprefab`", content_type = "application/zip"),
        (status = 403, body = Problem), (status = 404, body = Problem)))]
pub async fn export(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    let (name, bytes) = studio::export_archive(&pool, id).await?;
    Ok(crate::prefabs::http::archive_response(&name, bytes))
}

/// Publie le brouillon (même validation qu'une archive `.kprefab`), puis le supprime. Avec
/// `prefabs.publish_to_dir`, ses fichiers sont écrits dans le dossier des prefabs officiels et la
/// version entre en `official` ; sinon en `uploaded` (ADR-0042).
#[utoipa::path(post, path = "/api/admin/v1/studio/drafts/{id}/publish", tag = "admin-studio",
    security(("bearer" = [])), params(("id" = Uuid, Path)),
    responses((status = 201, body = PrefabDto), (status = 403, body = Problem), (status = 404, body = Problem),
        (status = 409, description = "Version déjà importée", body = Problem),
        (status = 422, description = "Prefab invalide (`params.reason`, `params.issues`)", body = Problem)))]
pub async fn publish(
    State(iam): State<IamState>,
    State(pool): State<PgPool>,
    State(servers): State<ServersState>,
    State(prefabs_dir): State<PrefabsDir>,
    session: AdminSession,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<PrefabDto>), ApiError> {
    session.require(&iam, Permission::PrefabsManage).await?;
    // L'essai en cours n'a plus d'objet : la version publiée le remplace. Relevé avant la
    // publication, qui efface le brouillon (et le lien des serveurs vers lui).
    let running = trials::running(&servers, id).await?;
    let imported = studio::publish(&pool, &session.context, &prefabs_dir, id).await?;
    trials::stop_all(
        &servers,
        &session.context,
        &running,
        id,
        EndReason::Published,
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(detail(&pool, &imported.id, &imported.version).await?),
    ))
}
