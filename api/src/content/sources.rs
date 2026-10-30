//! Sources de contenu de l'installation, par provider déclaré dans les prefabs.

use std::sync::Arc;

use kuro_prefab::manifest::Provider;

use super::provider::ContentProvider;

#[derive(Debug, Clone)]
pub struct ContentSources {
    pub modrinth: Arc<dyn ContentProvider>,
    pub curseforge: Arc<dyn ContentProvider>,
    pub steam: Arc<dyn ContentProvider>,
}

impl ContentSources {
    /// Source d'un provider de prefab ; aucune pour l'envoi de fichiers.
    pub fn get(&self, provider: Provider) -> Option<&Arc<dyn ContentProvider>> {
        match provider {
            Provider::Modrinth => Some(&self.modrinth),
            Provider::Curseforge => Some(&self.curseforge),
            Provider::SteamWorkshop => Some(&self.steam),
            Provider::Upload => None,
        }
    }
}

/// Page d'un projet chez sa source, d'après son identifiant : sa description, pour savoir ce
/// qu'il fait. CurseForge redirige `/projects/<id>` vers la page du mod.
pub fn project_page(source: &str, project_id: &str) -> Option<String> {
    let safe = !project_id.is_empty()
        && project_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !safe {
        return None;
    }
    match source {
        "modrinth" => Some(format!("https://modrinth.com/project/{project_id}")),
        "curseforge" => Some(format!("https://www.curseforge.com/projects/{project_id}")),
        "steam-workshop" => Some(super::steam::page_url(project_id)),
        _ => None,
    }
}

/// Nom de la source, tel que gardé avec le contenu installé et donné par l'API.
pub fn source_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Modrinth => "modrinth",
        Provider::Curseforge => "curseforge",
        Provider::Upload => "upload",
        Provider::SteamWorkshop => "steam-workshop",
    }
}
