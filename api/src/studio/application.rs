//! Cas d'usage du Prefab Studio : brouillons, validation pendant la saisie, images,
//! publication en version importée (même chemin qu'une archive `.kprefab`).

use std::collections::BTreeMap;
use std::path::Path;

use kuro_prefab::assets::{Asset, AssetKind, MAX_ASSET_BYTES, sniff};
use kuro_prefab::compose::{self, Conversion, NoteKind};
use kuro_prefab::egg;
use kuro_prefab::files::ArchiveFiles;
use kuro_prefab::package::{MANIFEST_FILE, parse_manifest, template_paths};
use kuro_prefab::{Issue, LoadError, Manifest, Package};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::prefabs::application::{self as prefabs, Imported, PrefabsError};
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
    /// Copie d'une version stockée, version suivante (`1.0.0` → `1.0.1`).
    From { id: String, version: String },
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
        Start::From { id, version } => {
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
            let assets = PrefabRepository::assets_of(&mut conn, id, version).await?;
            (
                bump_version(files, &manifest),
                Some((id.clone(), version.clone())),
                assets,
            )
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

/// Fichiers texte du dossier officiel de cette version, s'il est là.
fn official_files(prefabs_dir: &Path, id: &str, version: &str) -> Option<DraftFiles> {
    let dir = std::fs::read_dir(prefabs_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|dir| {
            std::fs::read_to_string(dir.join(MANIFEST_FILE))
                .ok()
                .and_then(|text| parse_manifest(&text).ok())
                .is_some_and(|manifest| {
                    manifest.id == id && manifest.version.to_string() == version
                })
        })?;
    let manifest_text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    let manifest = parse_manifest(&manifest_text).ok()?;
    let mut files = DraftFiles::from([(MANIFEST_FILE.to_owned(), manifest_text)]);
    if let Ok(entries) = std::fs::read_dir(dir.join("locales")) {
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            if let (Some(name), Ok(text)) = (
                path.file_name().and_then(|name| name.to_str()),
                std::fs::read_to_string(&path),
            ) {
                files.insert(format!("locales/{name}"), text);
            }
        }
    }
    for relative in template_paths(&manifest) {
        if let Ok(text) = std::fs::read_to_string(dir.join(&relative)) {
            files.insert(relative, text);
        }
    }
    Some(files)
}

/// Fichiers reconstitués depuis la version stockée (archive importée : fichiers d'origine non
/// conservés).
fn stored_files(
    manifest: &Manifest,
    locales: &Value,
    templates: &Value,
) -> Result<DraftFiles, PrefabsError> {
    let mut files = DraftFiles::from([(MANIFEST_FILE.to_owned(), manifest_yaml(manifest))]);
    if let Some(locales) = locales.as_object() {
        for (lang, messages) in locales {
            let text = serde_json::to_string_pretty(messages)
                .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
            files.insert(format!("locales/{lang}.json"), format!("{text}\n"));
        }
    }
    if let Some(templates) = templates.as_object() {
        for (path, text) in templates {
            if let Some(text) = text.as_str() {
                files.insert(path.clone(), text.to_owned());
            }
        }
    }
    Ok(files)
}

/// Manifest en YAML, sans les champs vides (`key: null`) que la sérialisation écrit. Si le
/// résultat ne se relit pas à l'identique, la version complète est gardée.
fn manifest_yaml(manifest: &Manifest) -> String {
    let full = serde_saphyr::to_string(manifest).unwrap_or_default();
    let mut lines: Vec<String> = full
        .lines()
        .filter(|line| !(line.trim_end().ends_with(": null") || line.trim_end().ends_with(": ~")))
        .map(str::to_owned)
        .collect();
    // Une clé dont tous les enfants ont disparu (`assets:` seul) part à son tour. Une liste
    // s'écrit au même retrait que sa clé (`tags:` puis `- a`) : ce n'est pas une clé vide.
    loop {
        let indent = |line: &str| line.len() - line.trim_start().len();
        let empty = (0..lines.len()).find(|&index| {
            let line = &lines[index];
            line.trim_end().ends_with(':')
                && lines.get(index + 1).is_none_or(|next| {
                    indent(next) < indent(line)
                        || (indent(next) == indent(line) && !next.trim_start().starts_with("- "))
                })
        });
        match empty {
            Some(index) => {
                lines.remove(index);
            }
            None => break,
        }
    }
    let compact = format!("{}\n", lines.join("\n"));
    match parse_manifest(&compact) {
        Ok(parsed) if &parsed == manifest => compact,
        _ => full,
    }
}

/// Passe le manifest à la version de correctif suivante.
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
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftUpdated).details(json!({
            "draft": id,
            "asset": kind.as_str(),
            "bytes": data.len(),
        })),
    )
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

/// Publie le brouillon en version importée (`uploaded`), par le même chemin qu'une archive :
/// même validation, version immuable, refus journalisé. Le brouillon disparaît ensuite.
pub async fn publish(
    pool: &PgPool,
    context: &AuditContext,
    id: Uuid,
) -> Result<Imported, PrefabsError> {
    let loaded = draft_package(pool, id).await?;
    let imported = prefabs::import_loaded(
        pool,
        context,
        loaded,
        json!({ "draft": id }),
        prefabs::ImportAs::Uploaded,
    )
    .await?;

    let mut tx = AuditedTx::begin(pool, context).await?;
    DraftRepository::delete(&mut tx, id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::PrefabDraftPublished).details(json!({
            "draft": id,
            "prefab": imported.id,
            "version": imported.version,
        })),
    )
    .await?;
    Ok(imported)
}
