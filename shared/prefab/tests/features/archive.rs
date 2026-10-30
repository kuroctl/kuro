//! Archives `.kprefab` : même chargement qu'un dossier, archive d'un dossier acceptée, et refus
//! des archives piégées (chemin hors du prefab, lien symbolique, fichier ou total trop gros, trop
//! de fichiers, pas de manifest).

use std::io::Write;
use std::path::Path;

use kuro_prefab::assets::{load_assets, load_assets_from};
use kuro_prefab::files::{MAX_ARCHIVE_ENTRIES, MAX_ENTRY_BYTES};
use kuro_prefab::{LoadError, Package};
use zip::write::SimpleFileOptions;

use crate::support::{official_dirs, prefabs_dir};

/// Archive des fichiers du prefab officiel `id`, sous `prefix` (vide : à la racine).
fn zip_official(id: &str, prefix: &str) -> Vec<u8> {
    let dir = prefabs_dir().join(id);
    let mut files = Vec::new();
    collect(&dir, &dir, &mut files);
    zip_files(
        files
            .iter()
            .map(|(name, data)| (format!("{prefix}{name}"), data.clone())),
    )
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((name, std::fs::read(&path).unwrap()));
        }
    }
}

fn zip_files(files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in files {
        writer.start_file(name, options).unwrap();
        writer.write_all(&data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn refused(bytes: &[u8]) -> LoadError {
    Package::load_zip(bytes).expect_err("archive accepted")
}

#[test]
fn an_archive_loads_exactly_like_its_folder() {
    for id in official_dirs() {
        let (package, files) = Package::load_zip(&zip_official(&id, "")).unwrap();
        let from_dir = Package::load_dir(&prefabs_dir().join(&id)).unwrap();
        assert_eq!(package, from_dir, "{id}");
        assert_eq!(
            load_assets_from(&files, &package.manifest).unwrap(),
            load_assets(&prefabs_dir().join(&id), &from_dir.manifest).unwrap(),
            "{id}"
        );
    }
}

#[test]
fn an_archive_of_the_folder_itself_is_accepted() {
    let (package, files) = Package::load_zip(&zip_official("terraria", "terraria/")).unwrap();
    assert_eq!(
        package,
        Package::load_dir(&prefabs_dir().join("terraria")).unwrap()
    );
    assert_eq!(
        load_assets_from(&files, &package.manifest).unwrap(),
        load_assets(&prefabs_dir().join("terraria"), &package.manifest).unwrap()
    );
}

#[test]
fn paths_leaving_the_prefab_are_refused() {
    let bytes = zip_files([
        ("prefab.yaml".to_owned(), b"id: x".to_vec()),
        ("../outside.txt".to_owned(), b"boom".to_vec()),
    ]);
    assert!(matches!(refused(&bytes), LoadError::Archive(_)));
}

#[test]
fn symbolic_links_are_refused() {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .add_symlink("prefab.yaml", "/etc/passwd", SimpleFileOptions::default())
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(matches!(refused(&bytes), LoadError::Archive(reason) if reason.contains("symbolic")));
}

#[test]
fn an_oversized_file_is_refused_even_if_well_compressed() {
    // Un fichier de zéros se compresse très bien : c'est sa taille décompressée qui compte.
    let bytes = zip_files([
        ("prefab.yaml".to_owned(), b"id: x".to_vec()),
        (
            "templates/big.txt".to_owned(),
            vec![0; usize::try_from(MAX_ENTRY_BYTES).unwrap() + 1],
        ),
    ]);
    assert!(bytes.len() < 64 * 1024);
    assert!(matches!(refused(&bytes), LoadError::TooLarge { .. }));
}

#[test]
fn too_many_files_are_refused() {
    let files = (0..=MAX_ARCHIVE_ENTRIES).map(|index| (format!("f{index}.txt"), b"x".to_vec()));
    assert!(matches!(refused(&zip_files(files)), LoadError::Archive(_)));
}

#[test]
fn an_archive_without_manifest_or_not_a_zip_is_refused() {
    let bytes = zip_files([("readme.md".to_owned(), b"hello".to_vec())]);
    assert!(matches!(refused(&bytes), LoadError::Archive(_)));
    assert!(matches!(
        refused(b"definitely not a zip"),
        LoadError::Archive(_)
    ));
}

#[test]
fn an_invalid_manifest_in_an_archive_reports_its_issues() {
    let bytes = zip_files([(
        "prefab.yaml".to_owned(),
        b"id: Not Valid\nversion: nope\n".to_vec(),
    )]);
    assert!(matches!(
        refused(&bytes),
        LoadError::Manifest(_) | LoadError::Invalid(_)
    ));
}

/// Prefab officiel en archive, avec une icône (`assets/icon.png`, déclarée par le manifest).
fn with_icon(icon: &[u8]) -> Vec<u8> {
    let dir = prefabs_dir().join("minecraft-java");
    let mut files = Vec::new();
    collect(&dir, &dir, &mut files);
    files.retain(|(name, _)| name != "assets/icon.png");
    files.push(("assets/icon.png".to_owned(), icon.to_vec()));
    zip_files(files)
}

#[test]
fn an_archive_brings_its_images_and_never_an_svg() {
    let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0; 16]].concat();
    let (package, files) = Package::load_zip(&with_icon(&png)).unwrap();
    let assets = load_assets_from(&files, &package.manifest).unwrap();
    let icon = assets
        .iter()
        .find(|asset| asset.kind == kuro_prefab::assets::AssetKind::Icon)
        .unwrap();
    assert_eq!(icon.content_type, "image/png");
    assert!(icon.data.starts_with(&[0x89, b'P', b'N', b'G']));

    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
    let (package, files) = Package::load_zip(&with_icon(svg)).unwrap();
    assert!(matches!(
        load_assets_from(&files, &package.manifest),
        Err(LoadError::Asset { .. })
    ));
}

/// Fichiers d'un brouillon du Studio : chargés comme une archive.
fn official_map(id: &str) -> std::collections::BTreeMap<String, Vec<u8>> {
    let dir = prefabs_dir().join(id);
    let mut files = Vec::new();
    collect(&dir, &dir, &mut files);
    files.into_iter().collect()
}

#[test]
fn files_in_memory_load_like_a_folder() {
    let files = kuro_prefab::files::ArchiveFiles::from_map(official_map("terraria")).unwrap();
    let package = Package::load(&files).unwrap();
    assert_eq!(
        package,
        Package::load_dir(&prefabs_dir().join("terraria")).unwrap()
    );
}

#[test]
fn files_in_memory_refuse_unsafe_paths_and_a_missing_manifest() {
    for name in [
        "../escape.json",
        "/etc/passwd",
        "locales//fr.json",
        r"locales\fr.json",
        "./x",
    ] {
        let mut files = official_map("terraria");
        files.insert(name.to_owned(), b"{}".to_vec());
        let error = kuro_prefab::files::ArchiveFiles::from_map(files).expect_err(name);
        assert!(matches!(error, LoadError::Archive(_)), "{name}: {error}");
    }
    let mut files = official_map("terraria");
    files.remove("prefab.yaml");
    assert!(kuro_prefab::files::ArchiveFiles::from_map(files).is_err());
    let mut files = official_map("terraria");
    files.insert("big.bin".into(), vec![0; MAX_ENTRY_BYTES as usize + 1]);
    assert!(matches!(
        kuro_prefab::files::ArchiveFiles::from_map(files),
        Err(LoadError::TooLarge { .. })
    ));
}
