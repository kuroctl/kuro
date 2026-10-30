//! Commandes de l'API sur le contenu des serveurs : mods, plugins, modpacks (ADR-0020).
//!
//! Les fichiers sont résolus par l'API (provider, version compatible, dépendances) ; le daemon
//! télécharge, vérifie l'empreinte publiée par la source (SHA-512 pour Modrinth, SHA-1 pour
//! CurseForge) et dépose sous le dossier de l'emplacement, sans rien décider.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Dépose des fichiers dans le dossier d'un emplacement ; acceptée tout de suite, terminée par
/// `content.done`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentInstall {
    pub server_id: Uuid,
    pub operation_id: Uuid,
    pub service: String,
    /// Dossier de l'emplacement, chemin absolu dans le conteneur, sous un volume du service.
    pub dir: String,
    pub files: Vec<ContentFile>,
    /// Dossier du jeu (modpack) : les `file_name` sont alors des chemins relatifs à lui
    /// (`mods/x.jar`), et l'archive y est extraite. Absent : `file_name` est un nom dans `dir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// Archive de modpack dont `overrides/` et `server-overrides/` sont extraits dans `root`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ContentFile>,
    /// Chaque fichier est une archive de paquets à extraire (ADR-0030, Bedrock) ; absent : les
    /// fichiers sont déposés tels quels dans `dir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<ContentExtract>,
}

/// Extraction en paquets : l'archive (`.mcaddon`, `.mcpack`, zip) est découpée en paquets, chacun
/// portant le fichier `describe` ; sa description choisit son dossier parmi `routes`. Le daemon
/// rapporte les paquets et leurs descriptions dans `content.done`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentExtract {
    /// Chemin du fichier de description dans un paquet (`manifest.json`), JSON.
    pub describe: String,
    pub routes: Vec<ExtractRoute>,
}

/// Valeur d'une description au chemin `path` (`modules.0.type`) : clés d'objet et index de
/// tableau séparés par des points.
pub fn lookup<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('.')
        .try_fold(value, |current, part| match current {
            serde_json::Value::Object(map) => map.get(part),
            serde_json::Value::Array(items) => items.get(part.parse::<usize>().ok()?),
            _ => None,
        })
}

/// Valeur comparée par une route ou une inscription : texte tel quel, nombre ou booléen en texte.
pub fn matches(value: &serde_json::Value, path: &str, expected: &str) -> bool {
    match lookup(value, path) {
        Some(serde_json::Value::String(text)) => text == expected,
        Some(serde_json::Value::Number(number)) => number.to_string() == expected,
        Some(serde_json::Value::Bool(flag)) => (if *flag { "true" } else { "false" }) == expected,
        _ => false,
    }
}

/// Le paquet va dans `dir` si la valeur `value` de sa description vaut `is`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractRoute {
    /// Chemin dans la description : clés et index séparés par des points (`modules.0.type`).
    pub value: String,
    pub is: String,
    /// Dossier absolu dans le conteneur, sous un volume du service.
    pub dir: String,
}

/// Fichier à télécharger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentFile {
    /// HTTPS seulement.
    pub url: String,
    /// Empreinte attendue.
    pub hash: ContentHash,
    pub size: u64,
    /// Nom du fichier dans `dir` (un seul segment), ou chemin relatif pour un fichier de modpack.
    pub file_name: String,
}

/// Empreinte hexadécimale attendue, dans l'algorithme que publie la source du fichier :
/// `{ "algo": "sha512", "value": "…" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "algo", content = "value", rename_all = "lowercase")]
pub enum ContentHash {
    Sha512(String),
    /// CurseForge ne publie que SHA-1 (et MD5).
    Sha1(String),
}

impl ContentHash {
    pub fn value(&self) -> &str {
        match self {
            Self::Sha512(value) | Self::Sha1(value) => value,
        }
    }

    /// Longueur et alphabet attendus pour l'algorithme.
    pub fn is_well_formed(&self) -> bool {
        let length = match self {
            Self::Sha512(_) => 128,
            Self::Sha1(_) => 40,
        };
        let value = self.value();
        value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
    }
}

/// Supprime des fichiers de l'emplacement (et leur variante `.disabled`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRemove {
    pub server_id: Uuid,
    pub service: String,
    pub dir: String,
    /// Comme pour [`ContentInstall::root`] : noms relatifs à ce dossier du jeu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    pub file_names: Vec<String>,
    /// Dossiers de paquets extraits (chemins absolus sous un volume du service), supprimés avec
    /// leur contenu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<String>,
}

/// Active (`x.jar`) ou désactive (`x.jar.disabled`) un fichier de l'emplacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentToggle {
    pub server_id: Uuid,
    pub service: String,
    pub dir: String,
    pub file_name: String,
    pub enabled: bool,
}
