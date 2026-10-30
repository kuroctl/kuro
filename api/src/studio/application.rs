//! Cas d'usage du Prefab Studio : brouillons, validation pendant la saisie, images,
//! publication en version importée (même chemin qu'une archive `.kprefab`).

use std::collections::BTreeMap;
use std::path::Path;

use kuro_prefab::assets::{Asset, AssetKind, MAX_ASSET_BYTES, sniff};
use kuro_prefab::compose::{self, Conversion, NoteKind};
use kuro_prefab::egg;
use kuro_prefab::files::ArchiveFiles;
use kuro_prefab::package::{MANIFEST_FILE, parse_manifest};
use kuro_prefab::{Issue, LoadError, Manifest, Package};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::content::steamgriddb::SteamGrid;
use crate::content::{ContentSources, ProviderError};
use crate::prefabs::application::{self as prefabs, Imported, PrefabsDir, PrefabsError};
use crate::prefabs::export::{self, StoredAsset, official_files, stored_files};
use crate::prefabs::infra::PrefabRepository;
use crate::studio::infra::{DraftRecord, DraftRepository, NewDraft};

/// Fichiers texte d'un brouillon : chemin relatif → contenu.
pub type DraftFiles = BTreeMap<String, String>;

const STARTER: [(&str, &str); 3] = [
    (MANIFEST_FILE, include_str!("starter/prefab.yaml")),
    ("locales/fr.json", include_str!("starter/locales/fr.json")),
    ("locales/en.json", include_str!("starter/locales/en.json")),
];

/// Résultat de la validation : vide si le prefab est importable tel quel.
#[derive(Debug, Default)]
pub struct Check {
    /// Refus global (YAML illisible, traduction invalide…), sans emplacement précis.
    pub reason: Option<String>,
    /// Problèmes par chemin du manifest (`services.server.image`).
    pub issues: Vec<Issue>,
    pub prefab_id: Option<String>,
    pub prefab_version: Option<String>,
}

impl Check {
    pub fn valid(&self) -> bool {
        self.reason.is_none() && self.issues.is_empty()
    }
}

/// Fichiers aux bornes d'une archive (chemins sûrs, tailles, `prefab.yaml` présent) ; refusés
/// sinon, avant toute écriture.
fn bounded(files: &DraftFiles) -> Result<ArchiveFiles, PrefabsError> {
    ArchiveFiles::from_map(
        files
            .iter()
            .map(|(path, text)| (path.clone(), text.as_bytes().to_vec()))
            .collect(),
    )
    .map_err(|error| {
        let (reason, issues) = prefabs::refusal(error);
        PrefabsError::Invalid { reason, issues }
    })
}

/// Valide les fichiers comme à l'import, sans rien écrire.
pub fn check(files: &DraftFiles) -> Result<Check, PrefabsError> {
    let archive = bounded(files)?;
    let mut check = Check::default();
    if let Some(manifest) = files
        .get(MANIFEST_FILE)
        .and_then(|text| parse_manifest(text).ok())
    {
        check.prefab_id = Some(manifest.id.clone());
        check.prefab_version = Some(manifest.version.to_string());
    }
    if let Err(error) = Package::load(&archive) {
        match error {
            LoadError::Invalid(issues) => check.issues = issues,
            other => check.reason = Some(other.to_string()),
        }
    }
    Ok(check)
}

fn files_json(files: &DraftFiles) -> Value {
    json!(files)
}

/// Fichiers stockés d'un brouillon.
pub fn files_of(record: &DraftRecord) -> DraftFiles {
    serde_json::from_value(record.files.clone()).unwrap_or_default()
}

/// Point de départ d'un brouillon.
#[derive(Debug)]
pub enum Start {
    /// Prefab d'exemple, à compléter.
    Blank,
    /// Copie d'une version stockée : la version suivante (`1.0.0` → `1.0.1`) du même prefab, ou
    /// un nouveau prefab `new_id` en `1.0.0` (ADR-0039) ; `assets` : reprendre ses images.
    From {
        id: String,
        version: String,
        assets: bool,
        new_id: Option<String>,
    },
    /// Conversion d'un `docker-compose.yml` (ce qui est sûr ; le reste est refusé et noté).
    Compose { text: String, id: Option<String> },
    /// Conversion d'un egg Pterodactyl (ADR-0027).
    Egg { text: String, id: Option<String> },
}

/// Convertit un `docker-compose.yml` sans rien enregistrer (aperçu de l'import).
pub fn convert_compose(text: &str, id: Option<&str>) -> Result<Conversion, PrefabsError> {
    compose::convert(text, id).map_err(|error| PrefabsError::Invalid {
        reason: error.to_string(),
        issues: Vec::new(),
    })
}

/// Convertit un egg Pterodactyl sans rien enregistrer (aperçu de l'import, ADR-0027).
pub fn convert_egg(text: &str, id: Option<&str>) -> Result<Conversion, PrefabsError> {
    egg::convert(text, id).map_err(|error| PrefabsError::Invalid {
        reason: error.to_string(),
        issues: Vec::new(),
    })
}

/// Crée un brouillon. Une copie reprend les fichiers d'origine quand la version vient du dossier
/// des prefabs officiels (commentaires compris), sinon le manifest stocké réécrit en YAML ; ses
/// images sont reprises.
pub async fn create(
    pool: &PgPool,
    context: &AuditContext,
    prefabs_dir: &Path,
    start: Start,
) -> Result<Uuid, PrefabsError> {
    let mut origin = Value::Null;
    let mut egg_origin = Value::Null;
    let (files, base, assets) = match &start {
        Start::Blank => (
            STARTER
                .iter()
                .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
                .collect::<DraftFiles>(),
            None,
            Vec::new(),
        ),
        Start::From {
            id,
            version,
            assets: with_assets,
            new_id,
        } => {
            if let Some(new_id) = new_id
                && (!kuro_prefab::template::is_slug(new_id) || new_id == id)
            {
                return Err(PrefabsError::Invalid {
                    reason: "invalid id for the new prefab".to_owned(),
                    issues: Vec::new(),
                });
            }
            let mut conn = pool.acquire().await?;
            let record = PrefabRepository::get(&mut conn, id, version)
                .await?
                .ok_or(PrefabsError::NotFound)?;
            let manifest: Manifest = serde_json::from_str(&record.manifest)
                .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
            // Lecture du dossier officiel hors du runtime.
            let official = tokio::task::spawn_blocking({
                let (dir, id, version) = (prefabs_dir.to_path_buf(), id.clone(), version.clone());
                move || official_files(&dir, &id, &version)
            })
            .await
            .ok()
            .flatten();
            let files = match official {
                Some(files) => files,
                None => stored_files(&manifest, &record.locales, &record.templates)?,
            };
            let assets = if *with_assets {
                PrefabRepository::assets_of(&mut conn, id, version).await?
            } else {
                Vec::new()
            };
            match new_id {
                // Nouveau prefab tiré d'un modèle : son identifiant, en 1.0.0, sans base.
                Some(new_id) => (retarget(files, &manifest, new_id), None, assets),
                None => (
                    bump_version(files, &manifest),
                    Some((id.clone(), version.clone())),
                    assets,
                ),
            }
        }
        Start::Compose { text, id } => {
            let conversion = convert_compose(text, id.as_deref())?;
            origin = json!({
                "source": "docker-compose",
                "refused": conversion.notes.iter().filter(|note| note.kind == NoteKind::Refused).count(),
            });
            (conversion.files, None, Vec::new())
        }
        Start::Egg { text, id } => {
            let conversion = convert_egg(text, id.as_deref())?;
            egg_origin = json!({
                "source": "pterodactyl-egg",
                "refused": conversion.notes.iter().filter(|note| note.kind == NoteKind::Refused).count(),
            });
            (conversion.files, None, Vec::new())
        }
    };
    let check = check(&files)?;
    let id = Uuid::now_v7();
    let stored = files_json(&files);
    let mut tx = AuditedTx::begin(pool, context).await?;
    DraftRepository::insert(
        &mut tx,
        &NewDraft {
            id,
            files: &stored,
            prefab_id: check.prefab_id.as_deref(),
            prefab_version: check.prefab_version.as_deref(),
            base_id: base.as_ref().map(|(id, _)| id.as_str()),
            base_version: base.as_ref().map(|(_, version)| version.as_str()),
            author: context.actor.id,
        },
    )
    .await?;
    for (kind, data) in &assets {
        if let Some(content_type) = sniff(data) {
            DraftRepository::put_asset(&mut tx, id, kind, content_type, data).await?;
        }
    }
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftCreated).details(json!({
            "draft": id,
            "from": base.map(|(id, version)| format!("{id}@{version}")),
            "compose": origin,
            "egg": egg_origin,
        })),
    )
    .await?;
    Ok(id)
}

/// Passe le manifest à la version de correctif suivante.
/// Nouveau prefab tiré d'un modèle (ADR-0039) : `id` remplacé, version remise à `1.0.0`, sur
/// les lignes du manifest qui les portent (le reste du fichier est gardé tel quel).
fn retarget(mut files: DraftFiles, manifest: &Manifest, new_id: &str) -> DraftFiles {
    if let Some(text) = files.get_mut(MANIFEST_FILE) {
        let current_id = manifest.id.clone();
        let current_version = manifest.version.to_string();
        let (mut id_done, mut version_done) = (false, false);
        *text = text
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                if !id_done
                    && line == trimmed
                    && trimmed.starts_with("id:")
                    && trimmed.contains(&current_id)
                {
                    id_done = true;
                    line.replacen(&current_id, new_id, 1)
                } else if !version_done
                    && line == trimmed
                    && trimmed.starts_with("version:")
                    && trimmed.contains(&current_version)
                {
                    version_done = true;
                    line.replacen(&current_version, "1.0.0", 1)
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
    }
    files
}

fn bump_version(mut files: DraftFiles, manifest: &Manifest) -> DraftFiles {
    let mut next = manifest.version.clone();
    next.patch += 1;
    next.pre = semver::Prerelease::EMPTY;
    if let Some(text) = files.get_mut(MANIFEST_FILE) {
        let current = manifest.version.to_string();
        let mut done = false;
        *text = text
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                if !done
                    && line == trimmed
                    && trimmed.starts_with("version:")
                    && trimmed.contains(&current)
                {
                    done = true;
                    line.replacen(&current, &next.to_string(), 1)
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        text.push('\n');
    }
    files
}

/// Enregistre les fichiers (un YAML invalide s'enregistre : c'est un brouillon) et rend la
/// validation.
pub async fn save(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
    files: &DraftFiles,
) -> Result<Check, PrefabsError> {
    let check = check(files)?;
    let mut tx = AuditedTx::begin(pool, context).await?;
    let updated = DraftRepository::update_files(
        &mut tx,
        id,
        &files_json(files),
        check.prefab_id.as_deref(),
        check.prefab_version.as_deref(),
        context.actor.id,
    )
    .await?;
    if !updated {
        return Err(PrefabsError::NotFound);
    }
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftUpdated).details(json!({
            "draft": id,
            "files": files.len(),
            "valid": check.valid(),
        })),
    )
    .await?;
    Ok(check)
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<DraftRecord, PrefabsError> {
    let mut conn = pool.acquire().await?;
    DraftRepository::get(&mut conn, id)
        .await?
        .ok_or(PrefabsError::NotFound)
}

pub async fn list(pool: &PgPool) -> Result<Vec<DraftRecord>, PrefabsError> {
    let mut conn = pool.acquire().await?;
    Ok(DraftRepository::list(&mut conn).await?)
}

pub async fn delete(pool: &PgPool, context: &AuditContext, id: Uuid) -> Result<(), PrefabsError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    if !DraftRepository::delete(&mut tx, id).await? {
        return Err(PrefabsError::NotFound);
    }
    tx.commit(AuditEvent::success(AuditAction::PrefabDraftDeleted).details(json!({ "draft": id })))
        .await?;
    Ok(())
}

/// Dépose une image : PNG, JPEG ou WebP reconnu au contenu, jamais SVG, 2 Mio au plus.
pub async fn put_asset(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
    kind: AssetKind,
    data: &[u8],
) -> Result<(), PrefabsError> {
    store_asset(pool, context, id, kind, data, None).await
}

/// D'où vient une image importée (ADR-0040).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageOrigin {
    /// Image de la communauté SteamGridDB : son identifiant et son auteur.
    SteamGridDb { id: u64, author: Option<String> },
    /// CDN du magasin Steam.
    Steam,
}

impl ImageOrigin {
    pub fn source(&self) -> &'static str {
        match self {
            Self::SteamGridDb { .. } => "steamgriddb",
            Self::Steam => "steam",
        }
    }
}

/// Images importées pour un jeu Steam (ADR-0039, ADR-0040).
#[derive(Debug)]
pub struct SteamImport {
    pub app: u32,
    /// Nom du jeu sur le magasin.
    pub name: String,
    pub imported: Vec<(AssetKind, ImageOrigin)>,
    /// Introuvables, ou refusées (format, taille) : l'image du brouillon reste.
    pub missing: Vec<AssetKind>,
}

#[derive(Debug, thiserror::Error)]
pub enum SteamImportError {
    #[error(transparent)]
    Prefabs(#[from] PrefabsError),
    /// Le manifest enregistré ne déclare pas `steam.app`.
    #[error("no steam game declared")]
    NoGame,
    #[error("unknown steam game")]
    GameNotFound,
    #[error("steam unavailable: {0}")]
    Unavailable(String),
}

/// Importe les images du jeu Steam déclaré par le manifest **enregistré** (`steam.app`) :
/// jaquette, bannière et icône. Pour chacune, SteamGridDB d'abord quand sa clé est réglée (la
/// mieux notée), sinon le CDN du magasin (icône de la communauté, sinon logo). Une panne de
/// SteamGridDB ne fait pas échouer l'import. Les images trouvées remplacent celles du brouillon,
/// les autres sont listées ; chacune est journalisée avec sa source.
pub async fn import_steam_assets(
    pool: &PgPool,
    context: &AuditContext,
    sources: &ContentSources,
    id: Uuid,
) -> Result<SteamImport, SteamImportError> {
    let record = get(pool, id).await?;
    let app = files_of(&record)
        .get(MANIFEST_FILE)
        .and_then(|text| parse_manifest(text).ok())
        .and_then(|manifest| manifest.steam)
        .map(|steam| steam.app)
        .filter(|app| *app > 0)
        .ok_or(SteamImportError::NoGame)?;
    let unavailable = |error: ProviderError| match error {
        ProviderError::NotFound => SteamImportError::GameNotFound,
        other => SteamImportError::Unavailable(other.to_string()),
    };
    let game = sources.steam_store.game(app).await.map_err(unavailable)?;
    // Tout est lu avant d'écrire : Steam injoignable en cours de route ne laisse pas un brouillon
    // à moitié changé.
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for (kind, candidates) in &game.images {
        let mut data = from_steamgriddb(sources.steam_grid.as_ref(), app, *kind).await;
        if data.is_none() {
            // Premier chemin trouvé (icône 184 px, sinon 32 px…).
            for path in candidates {
                match sources.steam_store.image(path).await {
                    Ok(bytes) => {
                        data = Some((bytes, ImageOrigin::Steam));
                        break;
                    }
                    Err(ProviderError::NotFound) => {}
                    Err(other) => return Err(SteamImportError::Unavailable(other.to_string())),
                }
            }
        }
        match data {
            Some((data, origin)) => found.push((*kind, data, origin)),
            None => missing.push(*kind),
        }
    }
    let mut imported = Vec::new();
    for (kind, data, origin) in found {
        match store_asset(pool, context, id, kind, &data, Some((app, &origin))).await {
            Ok(()) => imported.push((kind, origin)),
            Err(PrefabsError::Invalid { .. }) => missing.push(kind),
            Err(other) => return Err(other.into()),
        }
    }
    imported.sort_by_key(|(kind, _)| *kind);
    missing.sort();
    Ok(SteamImport {
        app,
        name: game.name,
        imported,
        missing,
    })
}

/// Meilleure image de SteamGridDB pour ce type, lisible et reconnue comme image ; `None` sans
/// clé, sans image convenable, ou si SteamGridDB ne répond pas (le magasin prend le relais).
async fn from_steamgriddb(
    grid: &dyn SteamGrid,
    app: u32,
    kind: AssetKind,
) -> Option<(Vec<u8>, ImageOrigin)> {
    if !grid.configured() {
        return None;
    }
    let images = match grid.images(app, kind).await {
        Ok(images) => images,
        Err(error) => {
            tracing::warn!(%error, app, kind = kind.as_str(), "steamgriddb unavailable, using steam");
            return None;
        }
    };
    for image in images {
        match grid.fetch(&image.url).await {
            Ok(bytes) if sniff(&bytes).is_some() => {
                return Some((
                    bytes,
                    ImageOrigin::SteamGridDb {
                        id: image.id,
                        author: image.author,
                    },
                ));
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(%error, id = image.id, "steamgriddb image skipped"),
        }
    }
    None
}

async fn store_asset(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
    kind: AssetKind,
    data: &[u8],
    origin: Option<(u32, &ImageOrigin)>,
) -> Result<(), PrefabsError> {
    let refused = |reason: &str| PrefabsError::Invalid {
        reason: reason.to_owned(),
        issues: Vec::new(),
    };
    if data.len() as u64 > MAX_ASSET_BYTES {
        return Err(refused("image larger than 2 MiB"));
    }
    let content_type = sniff(data).ok_or_else(|| refused("expected a PNG, JPEG or WebP image"))?;
    let mut tx = AuditedTx::begin(pool, context).await?;
    if !DraftRepository::lock(&mut tx, id).await? {
        return Err(PrefabsError::NotFound);
    }
    DraftRepository::put_asset(&mut tx, id, kind.as_str(), content_type, data).await?;
    DraftRepository::touch(&mut tx, id, context.actor.id).await?;
    let mut details = json!({
        "draft": id,
        "asset": kind.as_str(),
        "bytes": data.len(),
    });
    if let Some((app, origin)) = origin {
        details["steam_app"] = json!(app);
        details["source"] = json!(origin.source());
        if let ImageOrigin::SteamGridDb { id, author } = origin {
            details["steamgriddb_image"] = json!(id);
            details["author"] = json!(author);
        }
    }
    tx.commit(AuditEvent::success(AuditAction::PrefabDraftUpdated).details(details))
        .await?;
    Ok(())
}

pub async fn delete_asset(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
    kind: AssetKind,
) -> Result<(), PrefabsError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    if !DraftRepository::lock(&mut tx, id).await?
        || !DraftRepository::delete_asset(&mut tx, id, kind.as_str()).await?
    {
        return Err(PrefabsError::NotFound);
    }
    DraftRepository::touch(&mut tx, id, context.actor.id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftUpdated).details(json!({
            "draft": id,
            "asset_removed": kind.as_str(),
        })),
    )
    .await?;
    Ok(())
}

/// Image d'un brouillon : type de contenu et octets.
pub async fn asset(
    pool: &PgPool,
    id: Uuid,
    kind: AssetKind,
) -> Result<(String, Vec<u8>), PrefabsError> {
    let mut conn = pool.acquire().await?;
    DraftRepository::assets(&mut conn, id)
        .await?
        .into_iter()
        .find(|(stored, _, _)| stored == kind.as_str())
        .map(|(_, content_type, data)| (content_type, data))
        .ok_or(PrefabsError::NotFound)
}

/// Paquet du brouillon et ses images, chargés comme une archive ; l'erreur de chargement est
/// rendue telle quelle, pour être journalisée par l'import.
pub async fn draft_package(
    pool: &PgPool,
    id: Uuid,
) -> Result<Result<(Package, Vec<Asset>), LoadError>, PrefabsError> {
    let record = get(pool, id).await?;
    let files = files_of(&record);
    let assets = {
        let mut conn = pool.acquire().await?;
        DraftRepository::assets(&mut conn, id).await?
    }
    .into_iter()
    .filter_map(|(kind, _, data)| {
        Some(Asset {
            kind: AssetKind::parse(&kind)?,
            content_type: sniff(&data)?,
            data,
        })
    })
    .collect();
    Ok(bounded(&files)
        .map_err(|_| LoadError::Archive("draft exceeds the archive limits".to_owned()))
        .and_then(|archive| prefabs::load_package(&archive, assets)))
}

/// Publie le brouillon, par le même chemin qu'une archive : même validation, version immuable,
/// refus journalisé. Le brouillon disparaît ensuite.
///
/// Avec `prefabs.publish_to_dir` (ADR-0042), ses fichiers sont écrits dans le dossier des
/// prefabs officiels (celui qui porte déjà cet identifiant, sinon `<id>/`), images déclarées
/// comprises, puis le dossier est réimporté : la version entre au catalogue en `official`, prête
/// à être commitée dans le dépôt des prefabs. Sinon elle entre en `uploaded`.
pub async fn publish(
    pool: &PgPool,
    context: &AuditContext,
    prefabs_dir: &PrefabsDir,
    id: Uuid,
) -> Result<Imported, PrefabsError> {
    let record = get(pool, id).await?;
    let loaded = draft_package(pool, id).await?;
    let mut exported_to = Value::Null;
    let imported = if prefabs_dir.publish_to_dir {
        let (package, assets) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                let (reason, issues) = prefabs::refusal(error);
                return Err(PrefabsError::Invalid { reason, issues });
            }
        };
        let (prefab_id, version) = (
            package.manifest.id.clone(),
            package.manifest.version.to_string(),
        );
        {
            let mut conn = pool.acquire().await?;
            if PrefabRepository::get(&mut conn, &prefab_id, &version)
                .await?
                .is_some()
            {
                return Err(PrefabsError::Exists);
            }
        }
        let files = export::with_assets(
            files_of(&record),
            Some(&package.manifest),
            assets
                .into_iter()
                .map(|asset| StoredAsset {
                    kind: asset.kind,
                    data: asset.data,
                })
                .collect(),
        );
        let folder = tokio::task::spawn_blocking({
            let dir = prefabs_dir.dir.clone();
            let prefab_id = prefab_id.clone();
            move || {
                let folder = export::folder_for(&dir, &prefab_id);
                export::write_dir(&folder, &files).map(|()| folder)
            }
        })
        .await
        .map_err(|e| PrefabsError::Io(e.to_string()))?
        .map_err(|e| PrefabsError::Io(e.to_string()))?;
        exported_to = json!(
            folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        );
        let report = prefabs::import_dir_as(pool, context, &prefabs_dir.dir).await?;
        let label = format!("{prefab_id}@{version}");
        if !report.imported.contains(&label) {
            let reason = report
                .invalid
                .into_iter()
                .find(|(name, _)| {
                    folder
                        .file_name()
                        .is_some_and(|f| f.to_string_lossy() == *name)
                })
                .map(|(_, reason)| reason)
                .unwrap_or_else(|| "written prefab was not imported".to_owned());
            return Err(PrefabsError::Io(reason));
        }
        Imported {
            id: prefab_id,
            version,
        }
    } else {
        prefabs::import_loaded(
            pool,
            context,
            loaded,
            json!({ "draft": id }),
            prefabs::ImportAs::Uploaded,
        )
        .await?
    };

    let mut tx = AuditedTx::begin(pool, context).await?;
    DraftRepository::delete(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftPublished).details(json!({
            "draft": id,
            "prefab": imported.id,
            "version": imported.version,
            "exported_to": exported_to,
        })),
    )
    .await?;
    Ok(imported)
}

/// Archive `.kprefab` du brouillon (ADR-0042) : ses fichiers tels qu'enregistrés et ses images,
/// déclarées dans le manifest si elles ne l'étaient pas. Rend `(nom de fichier, zip)`. Un
/// manifest illisible est exporté tel quel, images dans `assets/`.
pub async fn export_archive(pool: &PgPool, id: Uuid) -> Result<(String, Vec<u8>), PrefabsError> {
    let record = get(pool, id).await?;
    let files = files_of(&record);
    let manifest = files
        .get(MANIFEST_FILE)
        .and_then(|text| parse_manifest(text).ok());
    let assets = {
        let mut conn = pool.acquire().await?;
        DraftRepository::assets(&mut conn, id).await?
    }
    .into_iter()
    .filter_map(|(kind, _, data)| {
        Some(StoredAsset {
            kind: AssetKind::parse(&kind)?,
            data,
        })
    })
    .collect();
    let (folder, name) = match &manifest {
        Some(manifest) => (
            manifest.id.clone(),
            export::archive_name(&manifest.id, &manifest.version.to_string()),
        ),
        None => (format!("draft-{id}"), format!("draft-{id}.kprefab")),
    };
    let files = export::with_assets(files, manifest.as_ref(), assets);
    Ok((name, export::zip(&folder, &files)?))
}
