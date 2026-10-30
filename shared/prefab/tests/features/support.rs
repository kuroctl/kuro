//! Outillage commun : prefabs officiels et copies modifiées pour les cas piégés.

use std::path::{Path, PathBuf};

use kuro_prefab::{LoadError, Package};
use serde_json::Value;
use tempfile::TempDir;

pub fn prefabs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prefabs")
}

pub fn official(id: &str) -> Package {
    Package::load_dir(&prefabs_dir().join(id)).unwrap()
}

/// Dossiers des prefabs officiels (nom libre ; l'`id` est celui du manifest).
pub const OFFICIAL: &[&str] = &[
    "minecraft-java",
    "wordpress",
    "ark-survival-ascended",
    "terraria",
    "valheim",
    "project-zomboid",
    "rust",
    "enshrouded",
    "minecraft-bedrock",
    "arma-3",
    "fivem-gta5",
    "fivem-red-dead-redemption-2",
];

/// Copie d'un prefab officiel avec `prefab.yaml` modifié.
pub struct Variant {
    dir: TempDir,
}

impl Variant {
    pub fn of(id: &str, edit: impl FnOnce(String) -> String) -> Self {
        let dir = TempDir::new().unwrap();
        copy_dir(&prefabs_dir().join(id), dir.path());
        let manifest = dir.path().join("prefab.yaml");
        let text = std::fs::read_to_string(&manifest).unwrap();
        let edited = edit(text.clone());
        assert_ne!(edited, text, "the edit did not change the manifest");
        std::fs::write(&manifest, edited).unwrap();
        Self { dir }
    }

    /// Copie à l'identique, pour modifier d'autres fichiers que le manifest.
    pub fn copy(id: &str) -> Self {
        let dir = TempDir::new().unwrap();
        copy_dir(&prefabs_dir().join(id), dir.path());
        Self { dir }
    }

    pub fn load(&self) -> Result<Package, LoadError> {
        Package::load_dir(self.dir.path())
    }

    pub fn manifest_json(&self) -> Value {
        manifest_json(self.dir.path())
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// `prefab.yaml` lu comme une valeur JSON, pour le JSON Schema.
pub fn manifest_json(dir: &Path) -> Value {
    let text = std::fs::read_to_string(dir.join("prefab.yaml")).unwrap();
    serde_saphyr::from_str(&text).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
