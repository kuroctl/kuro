//! Import et lecture du catalogue de prefabs.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use kuro_prefab::assets::{Asset, AssetKind, load_assets, load_assets_from};
use kuro_prefab::files::ArchiveFiles;
use kuro_prefab::media::{self, Measure};
use kuro_prefab::{Issue, LoadError, Manifest, Package};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::prefabs::infra::{NewPrefab, PrefabRecord, PrefabRepository};

#[derive(Debug, thiserror::Error)]
pub enum PrefabsError {
    #[error("unknown prefab")]
    NotFound,
    #[error("stored prefab is unreadable: {0}")]
    Corrupted(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// Archive ou prefab refusé : raison, et problèmes de validation par chemin du manifest.
    #[error("invalid prefab: {reason}")]
    Invalid { reason: String, issues: Vec<Issue> },
    /// Cette version existe déjà : `id` + `version` est immuable.
    #[error("prefab version already imported")]
    Exists,
    /// Des serveurs utilisent encore cette version.
    #[error("prefab version in use")]
    InUse,
    /// Écriture des fichiers d'un prefab (dossier des prefabs, archive) impossible.
    #[error("cannot write prefab files: {0}")]
    Io(String),
}

/// Résultat de l'import des prefabs officiels.
#[derive(Debug, Default)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub unchanged: Vec<String>,
    /// Prefab modifié sans changement de version : la version stockée est conservée.
    pub conflicts: Vec<String>,
    /// Images ajoutées à un prefab déjà présent (`id@version: icon, cover`).
    pub assets_added: Vec<String>,
    /// Dossier invalide, avec la raison.
    pub invalid: Vec<(String, String)>,
}

/// Importe chaque `<dir>/<id>/prefab.yaml` valide. Un prefab déjà stocké n'est jamais modifié ;
/// seules ses images manquantes sont ajoutées (cosmétiques, hors de l'immuabilité).
pub async fn import_dir(pool: &PgPool, dir: &Path) -> Result<ImportReport, PrefabsError> {
    import_dir_as(pool, &AuditContext::system(), dir).await
}

/// Dossier de prefab lu : (nom du dossier, prefab et images, ou l'erreur).
type LoadedFolder = (String, Result<(Package, Vec<Asset>), String>);

/// Lit chaque sous-dossier portant un manifest, dans l'ordre des noms.
fn load_folders(dir: &Path) -> Vec<LoadedFolder> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut folders: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join(kuro_prefab::package::MANIFEST_FILE).is_file())
        .collect();
    folders.sort();
    folders
        .into_iter()
        .map(|folder| {
            let name = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let loaded = Package::load_dir(&folder)
                .map_err(|e| e.to_string())
                .and_then(|package| {
                    let assets =
                        load_assets(&folder, &package.manifest).map_err(|e| e.to_string())?;
                    Ok((package, assets))
                });
            (name, loaded)
        })
        .collect()
}

/// Même import, au nom de l'acteur donné (rechargement demandé par un admin).
pub async fn import_dir_as(
    pool: &PgPool,
    context: &AuditContext,
    dir: &Path,
) -> Result<ImportReport, PrefabsError> {
    let mut report = ImportReport::default();
    // Lecture des dossiers (manifests, modèles, images) hors du runtime ; la base ensuite.
    let loaded = tokio::task::spawn_blocking({
        let dir = dir.to_path_buf();
        move || load_folders(&dir)
    })
    .await
    .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
    forget_packages();

    for (name, loaded) in loaded {
        let (package, assets) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                report.invalid.push((name, error));
                continue;
            }
        };
        let id = package.manifest.id.clone();
        let version = package.manifest.version.to_string();
        let label = format!("{id}@{version}");
        // Sérialisé depuis le modèle typé : ordre du fichier conservé (`IndexMap`).
        let manifest = serde_json::to_string(&package.manifest)
            .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
        let locales = json!(package.locales);
        let templates = json!(package.templates);

        let mut tx = AuditedTx::begin(pool, context).await?;
        match PrefabRepository::get(&mut tx, &id, &version).await? {
            Some(stored) => {
                let same = same_except_cosmetics(&stored.manifest, &manifest)
                    && stored.locales == locales
                    && stored.templates == templates;
                // Contenu différent : la version stockée est gardée telle quelle. Ses images
                // manquantes, cosmétiques, sont ajoutées quand même.
                let added = insert_assets(&mut tx, &id, &version, &assets).await?;
                if !added.is_empty() {
                    report
                        .assets_added
                        .push(format!("{label}: {}", added.join(", ")));
                    tx.commit(
                        AuditEvent::success(AuditAction::PrefabAssetsImported)
                            .details(json!({ "prefab": id, "version": version, "assets": added })),
                    )
                    .await?;
                }
                if same {
                    report.unchanged.push(label);
                } else {
                    report.conflicts.push(label);
                }
            }
            None => {
                let prefab = NewPrefab {
                    id: &id,
                    version: &version,
                    manifest: &manifest,
                    locales: &locales,
                    templates: &templates,
                    source: "official",
                    imported_by: None,
                    draft_id: None,
                    store: None,
                };
                PrefabRepository::insert(&mut tx, &prefab).await?;
                let added = insert_assets(&mut tx, &id, &version, &assets).await?;
                tx.commit(
                    AuditEvent::success(AuditAction::PrefabImported).details(json!({
                        "prefab": id,
                        "version": version,
                        "source": "official",
                        "assets": added,
                    })),
                )
                .await?;
                report.imported.push(label);
            }
        }
    }
    Ok(report)
}

/// Relit le dossier des prefabs officiels sans redémarrer l'API : nouvelles versions et images
/// manquantes importées, comme au démarrage. Journalisé au nom de l'admin.
pub async fn reload(
    pool: &PgPool,
    context: &AuditContext,
    dir: &Path,
) -> Result<ImportReport, PrefabsError> {
    let report = import_dir_as(pool, context, dir).await?;
    crate::audit::application::record(
        pool,
        context,
        AuditEvent::success(AuditAction::PrefabsReloaded).details(json!({
            "imported": report.imported,
            "assets_added": report.assets_added,
            "conflicts": report.conflicts,
            "invalid": report.invalid.iter().map(|(folder, _)| folder).collect::<Vec<_>>(),
        })),
    )
    .await?;
    Ok(report)
}

/// Empreinte du dossier (chemins, tailles, dates de modification) : change dès qu'un fichier
/// est ajouté, modifié ou retiré.
fn fingerprint(dir: &Path) -> Vec<(std::path::PathBuf, u64, Option<std::time::SystemTime>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            match entry.metadata() {
                Ok(meta) if meta.is_dir() => stack.push(path),
                Ok(meta) => files.push((path, meta.len(), meta.modified().ok())),
                Err(_) => {}
            }
        }
    }
    files.sort();
    files
}

/// Développement (`prefabs.watch`) : réimporte le dossier dès qu'il change, sans redémarrage.
/// L'état de référence est pris à l'appel (juste après l'import du démarrage), pas au premier
/// tour de la boucle.
pub fn watch_dir(
    pool: PgPool,
    dir: std::path::PathBuf,
    shutdown: tokio_util::sync::CancellationToken,
) -> impl std::future::Future<Output = ()> {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(2);
    let mut seen = fingerprint(&dir);
    async move {
        loop {
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(EVERY) => {}
            }
            let current = fingerprint(&dir);
            if current == seen {
                continue;
            }
            seen = current;
            match import_dir(&pool, &dir).await {
                Ok(report) => tracing::info!(
                    imported = ?report.imported,
                    assets_added = ?report.assets_added,
                    conflicts = ?report.conflicts,
                    invalid = report.invalid.len(),
                    "prefabs folder changed, reloaded"
                ),
                Err(error) => tracing::error!(%error, "prefabs reload failed"),
            }
        }
    }
}

/// Dossier des prefabs officiels (`prefabs.dir`), pour le rechargement depuis le panel, et
/// s'il reçoit les brouillons publiés (`prefabs.publish_to_dir`, ADR-0042).
#[derive(Debug, Clone)]
pub struct PrefabsDir {
    pub dir: std::path::PathBuf,
    /// « Publier » écrit le brouillon dans `dir` et l'importe en `official` (développement).
    pub publish_to_dir: bool,
}

/// Prefab importé depuis le panel.
#[derive(Debug)]
pub struct Imported {
    pub id: String,
    pub version: String,
}

/// Raison du refus et problèmes de validation (par chemin du manifest).
pub fn refusal(error: LoadError) -> (String, Vec<Issue>) {
    match error {
        LoadError::Invalid(issues) => ("invalid".to_owned(), issues),
        other => (other.to_string(), Vec::new()),
    }
}

/// Importe une archive `.kprefab` : même validation que les prefabs officiels, version
/// immuable. Un refus est journalisé (le contenu vient d'un admin, il peut être piégé).
pub async fn import_archive(
    pool: &PgPool,
    context: &AuditContext,
    bytes: &[u8],
) -> Result<Imported, PrefabsError> {
    let loaded = ArchiveFiles::from_zip(bytes).and_then(|files| load_package(&files, Vec::new()));
    import_loaded(
        pool,
        context,
        loaded,
        json!({ "bytes": bytes.len() }),
        ImportAs::Uploaded,
    )
    .await
}

/// Paquet et images lus depuis des fichiers en mémoire. Les images `extra` (téléversées à part,
/// dans le Studio) passent avant celles que déclare le manifest.
pub fn load_package(
    files: &ArchiveFiles,
    extra: Vec<Asset>,
) -> Result<(Package, Vec<Asset>), LoadError> {
    let package = Package::load(files)?;
    let mut assets = extra;
    for asset in load_assets_from(files, &package.manifest)? {
        if !assets.iter().any(|known| known.kind == asset.kind) {
            assets.push(asset);
        }
    }
    Ok((package, assets))
}

/// Nature d'une version importée depuis le panel.
#[derive(Debug, Clone)]
pub enum ImportAs {
    /// Version du catalogue (`uploaded`) : archive ou brouillon publié.
    Uploaded,
    /// Version d'essai cachée (`trial`) d'un brouillon, sous la version donnée
    /// (`1.0.1-trial.1`), hors du catalogue.
    Trial {
        draft: uuid::Uuid,
        version: semver::Version,
    },
    /// Version installée depuis un store (ADR-0042), nommé.
    Store { store: String },
}

/// Importe un paquet chargé (archive ou brouillon du Studio). `origin` complète l'événement
/// d'audit (taille de l'archive, brouillon publié ou essayé).
pub async fn import_loaded(
    pool: &PgPool,
    context: &AuditContext,
    loaded: Result<(Package, Vec<Asset>), LoadError>,
    origin: Value,
    import_as: ImportAs,
) -> Result<Imported, PrefabsError> {
    let (source, draft_id, store) = match &import_as {
        ImportAs::Uploaded => ("uploaded", None, None),
        ImportAs::Trial { draft, .. } => ("trial", Some(*draft), None),
        ImportAs::Store { store } => ("store", None, Some(store.as_str())),
    };
    let with_origin = |mut details: Value| {
        if let (Some(details), Some(origin)) = (details.as_object_mut(), origin.as_object()) {
            details.extend(origin.clone());
        }
        details
    };
    let (mut package, assets) = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            let (reason, issues) = refusal(error);
            crate::audit::application::record(
                pool,
                context,
                AuditEvent::failure(AuditAction::PrefabImported).details(with_origin(json!({
                    "source": source,
                    "reason": reason,
                    "issues": issues.len(),
                }))),
            )
            .await?;
            return Err(PrefabsError::Invalid { reason, issues });
        }
    };
    if let ImportAs::Trial { version, .. } = &import_as {
        package.manifest.version = version.clone();
    }
    let id = package.manifest.id.clone();
    let version = package.manifest.version.to_string();
    let manifest = serde_json::to_string(&package.manifest)
        .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
    let locales = json!(package.locales);
    let templates = json!(package.templates);

    let mut tx = AuditedTx::begin(pool, context).await?;
    if PrefabRepository::get(&mut tx, &id, &version)
        .await?
        .is_some()
    {
        return Err(PrefabsError::Exists);
    }
    let prefab = NewPrefab {
        id: &id,
        version: &version,
        manifest: &manifest,
        locales: &locales,
        templates: &templates,
        source,
        imported_by: context.actor.id,
        draft_id,
        store,
    };
    PrefabRepository::insert(&mut tx, &prefab).await?;
    let added = insert_assets(&mut tx, &id, &version, &assets).await?;
    tx.commit(
        AuditEvent::success(AuditAction::PrefabImported).details(with_origin(json!({
            "prefab": id,
            "version": version,
            "source": source,
            "assets": added,
        }))),
    )
    .await?;
    Ok(Imported { id, version })
}

/// Supprime les versions d'essai qu'aucun serveur n'utilise plus (essais terminés). Rendu :
/// les versions supprimées (`id@version`).
pub async fn delete_unused_trials(
    pool: &PgPool,
    context: &AuditContext,
) -> Result<Vec<String>, PrefabsError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    let unused = PrefabRepository::unused_trials(&mut tx).await?;
    if unused.is_empty() {
        return Ok(Vec::new());
    }
    let mut removed = Vec::new();
    for (id, version) in unused {
        PrefabRepository::delete(&mut tx, &id, &version).await?;
        removed.push(format!("{id}@{version}"));
    }
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDeleted).details(json!({ "trials": removed })),
    )
    .await?;
    Ok(removed)
}

/// Supprime une version que plus aucun serveur n'utilise. Un prefab officiel revient au
/// prochain démarrage de l'API (dossier `prefabs/`).
pub async fn delete(
    pool: &PgPool,
    context: &AuditContext,
    id: &str,
    version: &str,
) -> Result<(), PrefabsError> {
    let mut tx = AuditedTx::begin(pool, context).await?;
    if !PrefabRepository::lock(&mut tx, id, version).await? {
        return Err(PrefabsError::NotFound);
    }
    if !PrefabRepository::servers_of(&mut tx, id, version)
        .await?
        .is_empty()
    {
        return Err(PrefabsError::InUse);
    }
    PrefabRepository::delete(&mut tx, id, version).await?;
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDeleted)
            .details(json!({ "prefab": id, "version": version })),
    )
    .await?;
    forget_packages();
    Ok(())
}

/// Serveurs déployés depuis cette version : (id, nom, état).
pub async fn servers_of(
    pool: &PgPool,
    id: &str,
    version: &str,
) -> Result<Vec<(uuid::Uuid, String, String)>, PrefabsError> {
    let mut conn = pool.acquire().await?;
    PrefabRepository::get(&mut conn, id, version)
        .await?
        .ok_or(PrefabsError::NotFound)?;
    Ok(PrefabRepository::servers_of(&mut conn, id, version).await?)
}

/// Nombre de serveurs par (id, version).
pub async fn server_counts(pool: &PgPool) -> Result<HashMap<(String, String), i64>, PrefabsError> {
    let mut conn = pool.acquire().await?;
    Ok(PrefabRepository::server_counts(&mut conn)
        .await?
        .into_iter()
        .map(|(id, version, count)| ((id, version), count))
        .collect())
}

/// Même manifest aux rubriques `assets` et `steam` près : les images, et le jeu Steam d'où le
/// Studio les tire (ADR-0039), sont cosmétiques, hors de l'immuabilité de `id` + `version`
/// (ADR-0011). Les déclarer ne change donc pas la version ; le manifest stocké garde sa forme
/// d'origine.
///
/// Les deux côtés sont relus par le modèle actuel : un champ facultatif ajouté au modèle après
/// l'import (`backup`, `game_version`…) est absent du manifest stocké et vide dans l'actuel, sans
/// que le prefab ait changé.
fn same_except_cosmetics(stored: &str, current: &str) -> bool {
    let strip = |text: &str| {
        serde_json::from_str::<Manifest>(text)
            .ok()
            .and_then(|manifest| serde_json::to_value(manifest).ok())
            .or_else(|| serde_json::from_str::<Value>(text).ok())
            .map(|mut value| {
                if let Some(object) = value.as_object_mut() {
                    object.remove("assets");
                    object.remove("steam");
                }
                value
            })
    };
    match (strip(stored), strip(current)) {
        (Some(stored), Some(current)) => stored == current,
        _ => stored == current,
    }
}

/// Ajoute les images absentes ; retourne leurs types.
async fn insert_assets(
    conn: &mut PgConnection,
    id: &str,
    version: &str,
    assets: &[Asset],
) -> Result<Vec<&'static str>, sqlx::Error> {
    let mut added = Vec::new();
    for asset in assets {
        let kind = asset.kind.as_str();
        if PrefabRepository::insert_asset_if_absent(
            conn,
            id,
            version,
            kind,
            asset.content_type,
            &asset.data,
        )
        .await?
        {
            added.push(kind);
        }
    }
    Ok(added)
}

/// Image d'un prefab : type et octets.
pub async fn asset(
    pool: &PgPool,
    id: &str,
    version: &str,
    kind: AssetKind,
) -> Result<(String, Vec<u8>), PrefabsError> {
    let mut conn = pool.acquire().await?;
    PrefabRepository::asset(&mut conn, id, version, kind.as_str())
        .await?
        .ok_or(PrefabsError::NotFound)
}

/// Image remise aux dimensions convenues.
#[derive(Debug)]
pub struct OptimizedAsset {
    pub kind: AssetKind,
    pub before: Measure,
    pub after: Measure,
    /// Plus petite que la convention : jamais agrandie, à remplacer par une image plus grande.
    pub small: bool,
}

/// Résultat de l'optimisation des images d'une version.
#[derive(Debug, Default)]
pub struct OptimizeReport {
    pub optimized: Vec<OptimizedAsset>,
    /// Déjà conformes et optimisées.
    pub unchanged: Vec<AssetKind>,
    /// Illisibles ou trop lourdes une fois traitées : laissées telles quelles.
    pub failed: Vec<(AssetKind, String)>,
}

/// Met les images stockées d'une version aux dimensions convenues et les optimise (ADR-0011,
/// même traitement que `cargo xtask prefab-media`). Les images sont cosmétiques : les retoucher ne
/// touche pas à l'immuabilité de `id` + `version`. Journalisé au nom de l'admin.
pub async fn optimize_assets(
    pool: &PgPool,
    context: &AuditContext,
    id: &str,
    version: &str,
) -> Result<OptimizeReport, PrefabsError> {
    let stored = {
        let mut conn = pool.acquire().await?;
        if PrefabRepository::get(&mut conn, id, version)
            .await?
            .is_none()
        {
            return Err(PrefabsError::NotFound);
        }
        PrefabRepository::assets_of(&mut conn, id, version).await?
    };
    // Décodage et compression : plusieurs centaines de millisecondes par image, hors du runtime.
    let processed = tokio::task::spawn_blocking(move || {
        stored
            .into_iter()
            .filter_map(|(kind, data)| {
                AssetKind::parse(&kind).map(|kind| (kind, media::optimize(&data, kind)))
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| PrefabsError::Corrupted(error.to_string()))?;

    let mut report = OptimizeReport::default();
    let mut tx = AuditedTx::begin(pool, context).await?;
    for (kind, outcome) in processed {
        match outcome {
            Ok(Some(optimized)) => {
                PrefabRepository::replace_asset(
                    &mut tx,
                    id,
                    version,
                    kind.as_str(),
                    &optimized.data,
                )
                .await?;
                report.optimized.push(OptimizedAsset {
                    kind,
                    before: optimized.before,
                    after: optimized.after,
                    small: optimized.small,
                });
            }
            Ok(None) => report.unchanged.push(kind),
            Err(error) => report.failed.push((kind, error.to_string())),
        }
    }
    tx.commit(
        AuditEvent::success(AuditAction::PrefabAssetsOptimized).details(json!({
            "prefab": id,
            "version": version,
            "optimized": report.optimized.iter().map(|asset| json!({
                "kind": asset.kind.as_str(),
                "bytes_before": asset.before.bytes,
                "bytes_after": asset.after.bytes,
            })).collect::<Vec<_>>(),
            "failed": report.failed.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(),
        })),
    )
    .await?;
    Ok(report)
}

/// Types d'images présents, par (id, version).
pub async fn asset_kinds(
    pool: &PgPool,
) -> Result<HashMap<(String, String), Vec<String>>, PrefabsError> {
    let mut conn = pool.acquire().await?;
    let mut kinds: HashMap<(String, String), Vec<String>> = HashMap::new();
    for (id, version, kind) in PrefabRepository::asset_kinds(&mut conn).await? {
        kinds.entry((id, version)).or_default().push(kind);
    }
    Ok(kinds)
}

pub async fn list(pool: &PgPool) -> Result<Vec<PrefabRecord>, PrefabsError> {
    let mut conn = pool.acquire().await?;
    Ok(PrefabRepository::list(&mut conn).await?)
}

pub async fn get(pool: &PgPool, id: &str, version: &str) -> Result<PrefabRecord, PrefabsError> {
    let mut conn = pool.acquire().await?;
    PrefabRepository::get(&mut conn, id, version)
        .await?
        .ok_or(PrefabsError::NotFound)
}

/// Prefabs résolus, par base et (id, version). Une version importée ne change jamais de
/// contenu : la garder évite de relire et de désérialiser manifest, locales et modèles à chaque
/// déploiement ou réglage. Vidé à chaque import ou suppression faite par cette instance ; une
/// autre instance (ADR-0037) ne voit une suppression qu'au vidage suivant, sans conséquence (une
/// version supprimée n'a plus de serveur).
/// (base, id, version) → prefab résolu.
type PackageCache = HashMap<(String, String, String), Arc<Package>>;

static PACKAGES: LazyLock<Mutex<PackageCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));
const PACKAGE_CACHE_ENTRIES: usize = 256;

fn packages() -> std::sync::MutexGuard<'static, PackageCache> {
    PACKAGES.lock().unwrap_or_else(PoisonError::into_inner)
}

fn forget_packages() {
    packages().clear();
}

/// Base de la connexion : les tests font vivre plusieurs bases dans un même processus.
fn database_of(pool: &PgPool) -> String {
    pool.connect_options()
        .get_database()
        .unwrap_or_default()
        .to_owned()
}

/// Prefab stocké, prêt à résoudre un déploiement.
pub async fn package(pool: &PgPool, id: &str, version: &str) -> Result<Package, PrefabsError> {
    let key = (database_of(pool), id.to_owned(), version.to_owned());
    if let Some(package) = packages().get(&key) {
        return Ok(Package::clone(package));
    }
    let package = to_package(get(pool, id, version).await?)?;
    let mut cache = packages();
    if cache.len() >= PACKAGE_CACHE_ENTRIES {
        cache.clear();
    }
    cache.insert(key, Arc::new(package.clone()));
    Ok(package)
}

pub fn to_package(record: PrefabRecord) -> Result<Package, PrefabsError> {
    let corrupted = |e: serde_json::Error| PrefabsError::Corrupted(e.to_string());
    let manifest: Manifest = serde_json::from_str(&record.manifest).map_err(corrupted)?;
    let locales: BTreeMap<String, Value> =
        serde_json::from_value(record.locales).map_err(corrupted)?;
    let templates: BTreeMap<String, String> =
        serde_json::from_value(record.templates).map_err(corrupted)?;
    Ok(Package {
        manifest,
        locales,
        templates,
    })
}

/// Version marquée brouillon (`x-status: "draft: …"`) : image d'exécution à éprouver, réglages
/// incomplets… Elle ne se déploie que sur un nœud de test (ADR-0011), pour la mettre au point.
pub fn is_draft(manifest: &Manifest) -> bool {
    draft_reason(manifest).is_some()
}

/// Raison d'un brouillon, d'après `x-status: "draft: <raison>"` ; `Some("")` sans raison donnée.
pub fn draft_reason(manifest: &Manifest) -> Option<String> {
    let status = manifest.extensions.get("x-status")?.as_str()?;
    let rest = status.strip_prefix("draft")?;
    Some(rest.trim_start_matches(':').trim().to_owned())
}
