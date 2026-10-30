//! Export d'un prefab en fichiers (ADR-0042) : ceux d'un brouillon du Studio ou d'une version du
//! catalogue, prêts pour une archive `.kprefab` (zip avec un dossier de tête) ou pour le dossier
//! des prefabs officiels (`prefabs.publish_to_dir`).
//!
//! Une version officielle dont le dossier est présent est exportée avec ses fichiers d'origine
//! (commentaires compris). Une version importée n'a plus ses fichiers : son manifest est régénéré
//! depuis le modèle stocké, sans commentaires ni champs vides.

use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use kuro_prefab::Manifest;
use kuro_prefab::assets::{AssetKind, sniff};
use kuro_prefab::package::{MANIFEST_FILE, parse_manifest, template_paths};
use serde_json::Value;
use sqlx::PgPool;

use super::application::{self as prefabs, PrefabsError};
use super::infra::PrefabRepository;

/// Fichiers texte d'un prefab : chemin relatif (`locales/fr.json`) → contenu.
pub type TextFiles = BTreeMap<String, String>;
/// Tous les fichiers d'un prefab, images comprises : chemin relatif → octets.
pub type ExportFiles = BTreeMap<String, Vec<u8>>;

/// Image stockée à part de ses fichiers (base) : nature et octets.
#[derive(Debug, Clone)]
pub struct StoredAsset {
    pub kind: AssetKind,
    pub data: Vec<u8>,
}

/// Archive `.kprefab` d'une version du catalogue : `(nom de fichier, zip)`.
pub async fn version_archive(
    pool: &PgPool,
    prefabs_dir: &Path,
    id: &str,
    version: &str,
) -> Result<(String, Vec<u8>), PrefabsError> {
    let record = prefabs::get(pool, id, version).await?;
    let manifest: Manifest = serde_json::from_str(&record.manifest)
        .map_err(|e| PrefabsError::Corrupted(e.to_string()))?;
    // Lecture du dossier officiel hors du runtime.
    let official = tokio::task::spawn_blocking({
        let (dir, id, version) = (prefabs_dir.to_path_buf(), id.to_owned(), version.to_owned());
        move || official_files(&dir, &id, &version)
    })
    .await
    .ok()
    .flatten();
    let texts = match official {
        Some(files) => files,
        None => stored_files(&manifest, &record.locales, &record.templates)?,
    };
    let assets = {
        let mut conn = pool.acquire().await?;
        PrefabRepository::assets_of(&mut conn, id, version).await?
    }
    .into_iter()
    .filter_map(|(kind, data)| {
        Some(StoredAsset {
            kind: AssetKind::parse(&kind)?,
            data,
        })
    })
    .collect();
    let files = with_assets(texts, Some(&manifest), assets);
    Ok((archive_name(id, version), zip(id, &files)?))
}

/// Nom de fichier d'une archive : `<id>-<version>.kprefab`.
pub fn archive_name(id: &str, version: &str) -> String {
    format!("{id}-{version}.kprefab")
}

/// Fichiers texte du dossier officiel de cette version, s'il est là.
pub fn official_files(prefabs_dir: &Path, id: &str, version: &str) -> Option<TextFiles> {
    let dir = folder_of(prefabs_dir, |manifest| {
        manifest.id == id && manifest.version.to_string() == version
    })?;
    let manifest_text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    let manifest = parse_manifest(&manifest_text).ok()?;
    let mut files = TextFiles::from([(MANIFEST_FILE.to_owned(), manifest_text)]);
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

/// Dossier du prefab `id` sous le dossier des prefabs : celui dont le manifest porte cet
/// identifiant (son nom est libre), sinon `<dir>/<id>`.
pub fn folder_for(prefabs_dir: &Path, id: &str) -> PathBuf {
    folder_of(prefabs_dir, |manifest| manifest.id == id).unwrap_or_else(|| prefabs_dir.join(id))
}

/// Premier sous-dossier (par nom) dont le manifest se lit et satisfait `matches`.
fn folder_of(prefabs_dir: &Path, matches: impl Fn(&Manifest) -> bool) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(prefabs_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs.into_iter().find(|dir| {
        std::fs::read_to_string(dir.join(MANIFEST_FILE))
            .ok()
            .and_then(|text| parse_manifest(&text).ok())
            .is_some_and(|manifest| matches(&manifest))
    })
}

/// Fichiers reconstitués depuis la version stockée (archive importée : fichiers d'origine non
/// conservés).
pub fn stored_files(
    manifest: &Manifest,
    locales: &Value,
    templates: &Value,
) -> Result<TextFiles, PrefabsError> {
    let mut files = TextFiles::from([(MANIFEST_FILE.to_owned(), manifest_yaml(manifest))]);
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
pub fn manifest_yaml(manifest: &Manifest) -> String {
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

/// Fichiers texte et images réunis. Une image est écrite au chemin que le manifest déclare pour
/// sa nature ; une image non déclarée va dans `assets/<nature>.<ext>` et le manifest est complété
/// pour la déclarer (sans quoi un dossier ne l'importerait pas). Sans manifest lisible, les
/// images vont dans `assets/` telles quelles.
pub fn with_assets(
    texts: TextFiles,
    manifest: Option<&Manifest>,
    assets: Vec<StoredAsset>,
) -> ExportFiles {
    let mut files: ExportFiles = texts
        .iter()
        .map(|(path, text)| (path.clone(), text.as_bytes().to_vec()))
        .collect();
    let mut added: Vec<(AssetKind, String)> = Vec::new();
    for asset in assets {
        let declared = manifest.and_then(|manifest| declared_path(manifest, asset.kind));
        let path = match declared {
            Some(path) => path.to_owned(),
            None => {
                let path = format!(
                    "assets/{}.{}",
                    asset.kind.as_str(),
                    extension(sniff(&asset.data))
                );
                added.push((asset.kind, path.clone()));
                path
            }
        };
        files.insert(path, asset.data);
    }
    if let (Some(manifest), Some(text), false) =
        (manifest, texts.get(MANIFEST_FILE), added.is_empty())
    {
        files.insert(
            MANIFEST_FILE.to_owned(),
            declare_assets(text, manifest, &added).into_bytes(),
        );
    }
    files
}

fn declared_path(manifest: &Manifest, kind: AssetKind) -> Option<&str> {
    match kind {
        AssetKind::Icon => manifest.assets.icon.as_deref(),
        AssetKind::Cover => manifest.assets.cover.as_deref(),
        AssetKind::Thumbnail => manifest.assets.thumbnail.as_deref(),
    }
}

fn extension(content_type: Option<&str>) -> &'static str {
    match content_type {
        Some("image/jpeg") => "jpg",
        Some("image/webp") => "webp",
        _ => "png",
    }
}

/// Ajoute les déclarations d'images au texte du manifest, sous sa clé `assets:` ou à la fin, en
/// gardant le reste tel quel. Si le résultat ne se relit pas avec ces déclarations (clé en style
/// `{}`…), le manifest est régénéré en entier.
fn declare_assets(text: &str, manifest: &Manifest, added: &[(AssetKind, String)]) -> String {
    let declarations: Vec<String> = added
        .iter()
        .map(|(kind, path)| format!("  {}: {path}", kind.as_str()))
        .collect();
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    match lines.iter().position(|line| line.starts_with("assets:")) {
        Some(index) => {
            for (offset, declaration) in declarations.into_iter().enumerate() {
                lines.insert(index + 1 + offset, declaration);
            }
        }
        None => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("assets:".to_owned());
            lines.extend(declarations);
        }
    }
    let candidate = format!("{}\n", lines.join("\n"));
    let declares_all = |parsed: &Manifest| {
        added
            .iter()
            .all(|(kind, path)| declared_path(parsed, *kind) == Some(path.as_str()))
    };
    match parse_manifest(&candidate) {
        Ok(parsed) if declares_all(&parsed) => candidate,
        _ => {
            let mut full = manifest.clone();
            for (kind, path) in added {
                match kind {
                    AssetKind::Icon => full.assets.icon = Some(path.clone()),
                    AssetKind::Cover => full.assets.cover = Some(path.clone()),
                    AssetKind::Thumbnail => full.assets.thumbnail = Some(path.clone()),
                }
            }
            manifest_yaml(&full)
        }
    }
}

/// Archive zip des fichiers, sous un dossier de tête `<folder>/` (l'import retire ce dossier).
pub fn zip(folder: &str, files: &ExportFiles) -> Result<Vec<u8>, PrefabsError> {
    let io = |error: &dyn std::fmt::Display| PrefabsError::Io(error.to_string());
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (path, data) in files {
        writer
            .start_file(format!("{folder}/{path}"), options)
            .map_err(|e| io(&e))?;
        writer.write_all(data).map_err(|e| io(&e))?;
    }
    Ok(writer.finish().map_err(|e| io(&e))?.into_inner())
}

/// Écrit les fichiers dans `folder` (créé au besoin). Le manifest, les traductions et les gabarits
/// présents sont d'abord retirés, pour que le dossier reflète le prefab exporté ; les autres
/// fichiers (images d'une version précédente non redéclarées…) restent.
pub fn write_dir(folder: &Path, files: &ExportFiles) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    for stale in ["locales", "templates"] {
        let path = folder.join(stale);
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        }
    }
    let manifest = folder.join(MANIFEST_FILE);
    if manifest.is_file() {
        std::fs::remove_file(&manifest)?;
    }
    for (relative, data) in files {
        let path = folder.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    fn manifest(text: &str) -> Manifest {
        parse_manifest(text).unwrap()
    }

    /// Manifest d'exemple du Studio, sans images déclarées.
    const MINIMAL: &str = include_str!("../studio/starter/prefab.yaml");

    #[test]
    fn an_undeclared_image_is_declared_at_the_end_of_the_manifest() {
        let texts = TextFiles::from([(MANIFEST_FILE.to_owned(), MINIMAL.to_owned())]);
        let files = with_assets(
            texts,
            Some(&manifest(MINIMAL)),
            vec![StoredAsset {
                kind: AssetKind::Icon,
                data: PNG.to_vec(),
            }],
        );
        assert_eq!(files["assets/icon.png"], PNG);
        let text = String::from_utf8(files[MANIFEST_FILE].clone()).unwrap();
        assert!(text.starts_with(MINIMAL), "{text}");
        assert!(
            text.ends_with("\nassets:\n  icon: assets/icon.png\n"),
            "{text}"
        );
        assert_eq!(
            manifest(&text).assets.icon.as_deref(),
            Some("assets/icon.png")
        );
    }

    #[test]
    fn a_declared_image_keeps_its_path_and_a_flow_key_is_regenerated() {
        let declared = format!("{MINIMAL}assets:\n  cover: assets/banner.webp\n");
        let texts = TextFiles::from([(MANIFEST_FILE.to_owned(), declared.clone())]);
        let files = with_assets(
            texts,
            Some(&manifest(&declared)),
            vec![
                StoredAsset {
                    kind: AssetKind::Cover,
                    data: PNG.to_vec(),
                },
                StoredAsset {
                    kind: AssetKind::Icon,
                    data: PNG.to_vec(),
                },
            ],
        );
        assert!(files.contains_key("assets/banner.webp"));
        let text = String::from_utf8(files[MANIFEST_FILE].clone()).unwrap();
        assert!(
            text.contains("assets:\n  icon: assets/icon.png\n  cover: assets/banner.webp\n"),
            "{text}"
        );

        // Clé en style `{}` : l'insertion ligne à ligne ne se relit pas, le manifest est régénéré.
        let flow = format!("{MINIMAL}assets: {{}}\n");
        let texts = TextFiles::from([(MANIFEST_FILE.to_owned(), flow.clone())]);
        let files = with_assets(
            texts,
            Some(&manifest(&flow)),
            vec![StoredAsset {
                kind: AssetKind::Icon,
                data: PNG.to_vec(),
            }],
        );
        let text = String::from_utf8(files[MANIFEST_FILE].clone()).unwrap();
        let parsed = manifest(&text);
        assert_eq!(parsed.assets.icon.as_deref(), Some("assets/icon.png"));
        assert_eq!(parsed.id, "my-prefab");
    }

    #[test]
    fn the_archive_has_a_top_folder_and_imports_back() {
        let files = ExportFiles::from([
            (MANIFEST_FILE.to_owned(), MINIMAL.as_bytes().to_vec()),
            (
                "locales/en.json".to_owned(),
                include_bytes!("../studio/starter/locales/en.json").to_vec(),
            ),
        ]);
        let bytes = zip("my-prefab", &files).unwrap();
        let read = kuro_prefab::files::ArchiveFiles::from_zip(&bytes).unwrap();
        let (package, _) = prefabs::load_package(&read, Vec::new()).unwrap();
        assert_eq!(package.manifest.id, "my-prefab");
    }

    #[test]
    fn writing_a_folder_replaces_its_texts_and_keeps_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("demo");
        std::fs::create_dir_all(folder.join("templates")).unwrap();
        std::fs::write(folder.join("templates/old.cfg"), "old").unwrap();
        std::fs::create_dir_all(folder.join("assets")).unwrap();
        std::fs::write(folder.join("assets/cover.webp"), "img").unwrap();
        let files = ExportFiles::from([
            (MANIFEST_FILE.to_owned(), MINIMAL.as_bytes().to_vec()),
            ("templates/new.cfg".to_owned(), b"new".to_vec()),
        ]);
        write_dir(&folder, &files).unwrap();
        assert!(!folder.join("templates/old.cfg").exists());
        assert_eq!(
            std::fs::read_to_string(folder.join("templates/new.cfg")).unwrap(),
            "new"
        );
        assert!(folder.join("assets/cover.webp").exists());
        assert_eq!(folder_for(dir.path(), "demo"), folder);
        assert_eq!(folder_for(dir.path(), "other"), dir.path().join("other"));
    }
}
