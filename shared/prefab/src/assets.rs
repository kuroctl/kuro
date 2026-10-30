//! Images d'un prefab : icône (carrée) et bannière (`assets.icon`, `assets.cover`).
//!
//! Purement cosmétiques : un fichier déclaré mais absent est ignoré, un prefab publié reste
//! importable sans. Un fichier présent doit être une image PNG, JPEG ou WebP, reconnue à son
//! contenu et non à son extension : jamais de SVG, qui peut porter du script.

use std::path::{Path, PathBuf};

use crate::files::{DirFiles, PrefabFiles};
use crate::manifest::Manifest;
use crate::package::LoadError;
use crate::validate;

pub const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    Icon,
    Cover,
    /// Portrait (jaquette), pour les cartes du catalogue.
    Thumbnail,
}

impl AssetKind {
    pub const ALL: [Self; 3] = [Self::Icon, Self::Cover, Self::Thumbnail];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Cover => "cover",
            Self::Thumbnail => "thumbnail",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }

    fn declared(self, manifest: &Manifest) -> Option<&str> {
        match self {
            Self::Icon => manifest.assets.icon.as_deref(),
            Self::Cover => manifest.assets.cover.as_deref(),
            Self::Thumbnail => manifest.assets.thumbnail.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub kind: AssetKind,
    /// Type reconnu au contenu (`image/png`…).
    pub content_type: &'static str,
    pub data: Vec<u8>,
}

/// Images déclarées par le manifest et présentes dans le dossier du prefab.
pub fn load_assets(dir: &Path, manifest: &Manifest) -> Result<Vec<Asset>, LoadError> {
    load_assets_from(&DirFiles(dir), manifest)
}

/// Images déclarées par le manifest et présentes parmi ses fichiers (dossier ou archive).
pub fn load_assets_from(
    files: &impl PrefabFiles,
    manifest: &Manifest,
) -> Result<Vec<Asset>, LoadError> {
    let mut assets = Vec::new();
    for kind in AssetKind::ALL {
        let Some(relative) = kind.declared(manifest) else {
            continue;
        };
        // Chemin déjà refusé par la validation du manifest s'il sort du prefab.
        if !validate::is_prefab_path(relative) {
            continue;
        }
        let Some(data) = files.read(relative, MAX_ASSET_BYTES)? else {
            continue;
        };
        let content_type = sniff(&data).ok_or(LoadError::Asset {
            path: PathBuf::from(relative),
            reason: "expected a PNG, JPEG or WebP image",
        })?;
        assets.push(Asset {
            kind,
            content_type,
            data,
        });
    }
    Ok(assets)
}

/// Type d'image d'après ses premiers octets.
pub fn sniff(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_recognised_by_content() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    }

    #[test]
    fn svg_and_other_files_are_refused() {
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), None);
        assert_eq!(sniff(b"RIFF\0\0\0\0WAVE"), None);
        assert_eq!(sniff(b""), None);
    }

    #[test]
    fn kinds_round_trip_through_their_name() {
        for kind in AssetKind::ALL {
            assert_eq!(AssetKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(AssetKind::parse("banner"), None);
    }
}
