//! Index d'un store de prefabs (ADR-0042) : la liste des versions publiées par un dépôt de
//! prefabs, avec l'archive `.kprefab` de chacune et sa somme SHA-256. L'index est signé
//! (minisign) par qui le publie ; l'API le vérifie avant de s'y fier, et vérifie la somme de
//! chaque archive téléchargée contre l'index.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Package;

/// Format de l'index.
pub const INDEX_SCHEMA: &str = "kuro.prefab-index/v1";
/// Nom du fichier d'index à la racine d'un store, et de sa signature (`index.json.minisig`).
pub const INDEX_FILE: &str = "index.json";
/// Dossier des archives dans un store (`archives/<id>-<version>.kprefab`).
pub const ARCHIVES_DIR: &str = "archives";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreIndex {
    /// Toujours [`INDEX_SCHEMA`].
    pub schema: String,
    /// Date de génération (RFC 3339), informative.
    pub generated_at: String,
    /// Une entrée par version publiée, triées par `id` puis version.
    pub prefabs: Vec<IndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexEntry {
    pub id: String,
    pub version: String,
    pub category: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Langue → nom (`prefab.name` des traductions).
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    /// Langue → description.
    #[serde(default)]
    pub descriptions: BTreeMap<String, String>,
    /// `requires.kuro` du manifest : versions de Kuro acceptées (semver), si déclaré.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_kuro: Option<String>,
    /// Brouillon (`x-status: "draft: …"`) : ce qui reste à vérifier, tel que l'auteur l'a écrit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<String>,
    /// Images embarquées : `icon`, `cover`, `thumbnail`.
    #[serde(default)]
    pub assets: Vec<String>,
    /// Chemin de l'archive, relatif à la racine du store (`archives/wordpress-1.2.0.kprefab`).
    pub archive: String,
    /// SHA-256 de l'archive, en hexadécimal.
    pub sha256: String,
    /// Taille de l'archive en octets.
    pub size: u64,
}

impl StoreIndex {
    pub fn new(generated_at: String, mut prefabs: Vec<IndexEntry>) -> Self {
        prefabs.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.version.cmp(&b.version)));
        Self {
            schema: INDEX_SCHEMA.to_owned(),
            generated_at,
            prefabs,
        }
    }

    /// Lit et vérifie le format d'un index.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let index: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if index.schema != INDEX_SCHEMA {
            return Err(format!("unsupported index schema {}", index.schema));
        }
        for entry in &index.prefabs {
            if !crate::validate::is_prefab_path(&entry.archive) {
                return Err(format!("unsafe archive path {}", entry.archive));
            }
            if entry.sha256.len() != 64 || !entry.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("invalid sha256 for {}@{}", entry.id, entry.version));
            }
        }
        Ok(index)
    }

    pub fn find(&self, id: &str, version: &str) -> Option<&IndexEntry> {
        self.prefabs
            .iter()
            .find(|entry| entry.id == id && entry.version == version)
    }
}

impl IndexEntry {
    /// Entrée d'un prefab chargé, pour l'archive `archive` (`sha256` et `size` de ses octets).
    pub fn describe(
        package: &Package,
        assets: &[crate::assets::Asset],
        archive: String,
        sha256: String,
        size: u64,
    ) -> Self {
        let by_lang = |key: &str| -> BTreeMap<String, String> {
            package
                .locales
                .iter()
                .filter_map(|(lang, messages)| {
                    let text = messages.get("prefab")?.get(key)?.as_str()?;
                    Some((lang.clone(), text.to_owned()))
                })
                .collect()
        };
        let manifest = &package.manifest;
        Self {
            id: manifest.id.clone(),
            version: manifest.version.to_string(),
            category: serde_json::to_value(manifest.category)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            tags: manifest.tags.clone(),
            names: by_lang("name"),
            descriptions: by_lang("description"),
            requires_kuro: manifest.requires.kuro.clone(),
            draft: draft_reason(manifest),
            assets: assets
                .iter()
                .map(|asset| asset.kind.as_str().to_owned())
                .collect(),
            archive,
            sha256,
            size,
        }
    }

    /// Le prefab accepte cette version de Kuro (`requires.kuro` absent ou illisible : oui, le
    /// manifest l'aura dit à l'import).
    pub fn compatible_with(&self, kuro: &semver::Version) -> bool {
        compatible(self.requires_kuro.as_deref(), kuro)
    }
}

/// `requires.kuro` (`>=0.2`, `^1`) satisfait par `kuro`. Une exigence absente ou illisible
/// n'exclut rien : la validation du manifest est juge.
pub fn compatible(requires: Option<&str>, kuro: &semver::Version) -> bool {
    requires
        .and_then(|req| semver::VersionReq::parse(req).ok())
        .is_none_or(|req| req.matches(kuro))
}

/// Ce qui manque à un brouillon (`x-status: "draft: …"`), ou `None` pour une version publiée.
pub fn draft_reason(manifest: &crate::Manifest) -> Option<String> {
    let status = manifest.extensions.get("x-status")?.as_str()?;
    let rest = status.strip_prefix("draft")?;
    Some(rest.trim_start_matches(':').trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(requires: Option<&str>) -> IndexEntry {
        IndexEntry {
            id: "demo".into(),
            version: "1.0.0".into(),
            category: "game".into(),
            tags: vec![],
            names: BTreeMap::new(),
            descriptions: BTreeMap::new(),
            requires_kuro: requires.map(str::to_owned),
            draft: None,
            assets: vec![],
            archive: "archives/demo-1.0.0.kprefab".into(),
            sha256: "0".repeat(64),
            size: 1,
        }
    }

    #[test]
    fn the_index_round_trips_and_is_sorted() {
        let mut b = entry(None);
        b.id = "zzz".into();
        let index = StoreIndex::new("2026-10-30T00:00:00Z".into(), vec![b, entry(None)]);
        let bytes = serde_json::to_vec(&index).unwrap();
        let parsed = StoreIndex::parse(&bytes).unwrap();
        assert_eq!(parsed, index);
        assert_eq!(parsed.prefabs[0].id, "demo");
        assert!(parsed.find("zzz", "1.0.0").is_some());
    }

    #[test]
    fn a_bad_schema_path_or_digest_is_refused() {
        let mut index = StoreIndex::new("now".into(), vec![entry(None)]);
        index.schema = "other".into();
        assert!(StoreIndex::parse(&serde_json::to_vec(&index).unwrap()).is_err());
        let mut index = StoreIndex::new("now".into(), vec![entry(None)]);
        index.prefabs[0].archive = "../evil.kprefab".into();
        assert!(StoreIndex::parse(&serde_json::to_vec(&index).unwrap()).is_err());
        let mut index = StoreIndex::new("now".into(), vec![entry(None)]);
        index.prefabs[0].sha256 = "abc".into();
        assert!(StoreIndex::parse(&serde_json::to_vec(&index).unwrap()).is_err());
    }

    #[test]
    fn compatibility_follows_requires_kuro() {
        let kuro = semver::Version::new(0, 2, 0);
        assert!(entry(None).compatible_with(&kuro));
        assert!(entry(Some(">=0.2")).compatible_with(&kuro));
        assert!(!entry(Some(">=0.3")).compatible_with(&kuro));
        assert!(entry(Some("not a req")).compatible_with(&kuro));
    }
}
