//! Interface d'un provider de contenu et ses types, indépendants de la source.

use std::collections::HashMap;
use std::fmt::Debug;

use futures_util::future::BoxFuture;
use kuro_protocol::api::ContentHash;

/// Ce qui rend une version compatible avec un serveur.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Compat {
    /// `mod`, `plugin`, `modpack`.
    pub project_type: String,
    /// Jeu et classe de contenu chez la source (`minecraft`, `mods`), déclarés par le prefab :
    /// CurseForge en a besoin, Modrinth n'a qu'un jeu.
    pub game: Option<String>,
    pub class: Option<String>,
    /// `fabric`, `paper`… ; absent : tout loader. Version du jeu vide : toutes (modpack).
    pub loader: Option<String>,
    /// Version du jeu figée (`1.21.4`).
    pub game_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchQuery {
    pub text: String,
    pub compat: Compat,
    pub offset: u32,
    pub limit: u32,
    pub sort: SearchSort,
    /// Catégories de la source (identifiants qu'elle donne) : un résultat les a toutes.
    pub categories: Vec<String>,
}

/// Ordre des résultats, commun aux sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SearchSort {
    /// Pertinence pour le texte cherché ; sans texte, popularité.
    #[default]
    Relevance,
    Downloads,
    /// Dernière mise à jour.
    Updated,
    /// Première publication.
    Newest,
}

impl SearchSort {
    pub const ALL: [Self; 4] = [
        Self::Relevance,
        Self::Downloads,
        Self::Updated,
        Self::Newest,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Downloads => "downloads",
            Self::Updated => "updated",
            Self::Newest => "newest",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.as_str() == text)
    }
}

/// Catégorie de contenu chez une source, pour filtrer la recherche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    /// Identifiant à renvoyer dans [`SearchQuery::categories`] (nom chez Modrinth, numéro chez
    /// CurseForge).
    pub id: String,
    pub name: String,
    /// Icône SVG (Modrinth) : affichée comme masque CSS, jamais insérée dans la page.
    pub icon_svg: Option<String>,
    /// Icône en image (CurseForge).
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub downloads: u64,
    /// Page du projet chez la source.
    pub page_url: Option<String>,
    /// L'auteur autorise le téléchargement par des tiers (CurseForge) ; sinon, la page seule.
    pub download_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPage {
    pub hits: Vec<ProjectHit>,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderFile {
    pub url: String,
    pub file_name: String,
    /// Empreinte publiée par la source, vérifiée par le nœud.
    pub hash: ContentHash,
    pub size: u64,
    pub primary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub project_id: Option<String>,
    /// Version imposée par la dépendance ; sinon la plus récente compatible.
    pub version_id: Option<String>,
    /// Obligatoire (`required`) ; les autres (`optional`, `incompatible`…) sont ignorées.
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub published: String,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
    pub files: Vec<ProviderFile>,
    pub dependencies: Vec<Dependency>,
}

impl ProviderVersion {
    /// Fichier principal de la version.
    pub fn primary_file(&self) -> Option<&ProviderFile> {
        self.files
            .iter()
            .find(|file| file.primary)
            .or_else(|| self.files.first())
    }

    pub fn is_compatible(&self, compat: &Compat) -> bool {
        (compat.game_version.is_empty() || self.game_versions.contains(&compat.game_version))
            && compat.loader.as_deref().is_none_or(|loader| {
                accepted_loaders(loader)
                    .iter()
                    .any(|accepted| self.loaders.iter().any(|l| l == accepted))
            })
    }
}

/// Loaders dont un serveur peut charger le contenu : Quilt charge aussi les mods Fabric, publiés
/// le plus souvent pour Fabric seul.
pub fn accepted_loaders(loader: &str) -> Vec<&str> {
    match loader {
        "quilt" => vec!["quilt", "fabric"],
        other => vec![other],
    }
}

/// Projet : nom et icône, pour les dépendances ajoutées sans recherche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub icon_url: Option<String>,
    /// Description, où chercher les valeurs proposées d'un champ (ADR-0030).
    pub description: Option<String>,
    /// Voir [`ProjectHit::download_allowed`].
    pub download_allowed: bool,
}

/// Contenu installé depuis la source, pour chercher ses mises à jour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledRef {
    pub project_id: String,
    pub sha512: Option<String>,
}

/// Loader connu de la source (`fabric`, `paper`…) et son icône.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loader {
    pub name: String,
    /// Icône SVG fournie par la source ; le panel ne l'affiche que comme masque CSS, jamais
    /// insérée dans la page.
    pub icon_svg: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("not found")]
    NotFound,
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    /// Clé d'API refusée par la source.
    #[error("provider refused the api key")]
    Unauthorized,
    /// Source à clé d'API sans clé configurée.
    #[error("provider not configured")]
    NotConfigured,
}

/// Source de contenu. Toutes les réponses sont déjà filtrées par compatibilité quand une
/// `Compat` est donnée.
pub trait ContentProvider: Send + Sync + Debug {
    fn search(&self, query: &SearchQuery) -> BoxFuture<'_, Result<SearchPage, ProviderError>>;

    fn project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, ProviderError>>;

    /// Versions compatibles d'un projet, de la plus récente à la plus ancienne.
    fn versions(
        &self,
        project_id: &str,
        compat: &Compat,
    ) -> BoxFuture<'_, Result<Vec<ProviderVersion>, ProviderError>>;

    fn version(&self, version_id: &str) -> BoxFuture<'_, Result<ProviderVersion, ProviderError>>;

    /// Version compatible la plus récente de chaque contenu installé, par projet.
    fn latest(
        &self,
        installed: &[InstalledRef],
        compat: &Compat,
    ) -> BoxFuture<'_, Result<HashMap<String, ProviderVersion>, ProviderError>>;

    /// Dernière version stable du jeu, pour figer une version `LATEST`.
    fn latest_game_version(&self) -> BoxFuture<'_, Result<String, ProviderError>>;

    /// URL de fichier servie par une source autorisée (ADR-0020).
    fn is_allowed_download(&self, url: &str) -> bool;

    /// Hôtes que le format de modpack autorise pour ses fichiers.
    fn is_allowed_pack_download(&self, url: &str) -> bool;

    /// Fichier d'une source autorisée, `max_bytes` au plus (archive de modpack).
    fn fetch(&self, url: &str, max_bytes: usize) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>>;

    /// Loaders et leurs icônes, pour l'affichage. Aucun si la source n'en décrit pas.
    fn loaders(&self) -> BoxFuture<'_, Result<Vec<Loader>, ProviderError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Catégories qui filtrent la recherche pour ce type de contenu. Aucune si la source n'en a
    /// pas.
    fn categories(&self, _compat: &Compat) -> BoxFuture<'_, Result<Vec<Category>, ProviderError>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Source utilisable : une source à clé d'API en a une.
    fn configured(&self) -> bool {
        true
    }

    /// Clé d'API de l'installation (`None` : retirée). Sans effet pour une source ouverte.
    fn set_key(&self, _key: Option<String>) {}

    /// Vérifie une clé d'API auprès de la source, sans la garder.
    fn verify_key(&self, _key: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(loaders: &[&str]) -> ProviderVersion {
        ProviderVersion {
            id: "v1".to_owned(),
            project_id: "p1".to_owned(),
            name: "Mod".to_owned(),
            version_number: "1.0.0".to_owned(),
            published: String::new(),
            loaders: loaders.iter().map(|l| (*l).to_owned()).collect(),
            game_versions: vec!["1.21.4".to_owned()],
            files: Vec::new(),
            dependencies: Vec::new(),
        }
    }

    fn on(loader: &str) -> Compat {
        Compat {
            project_type: "mod".to_owned(),
            game: None,
            class: None,
            loader: Some(loader.to_owned()),
            game_version: "1.21.4".to_owned(),
        }
    }

    #[test]
    fn a_quilt_server_also_loads_fabric_mods() {
        assert!(version(&["quilt"]).is_compatible(&on("quilt")));
        assert!(version(&["fabric"]).is_compatible(&on("quilt")));
        assert!(!version(&["forge"]).is_compatible(&on("quilt")));
    }

    #[test]
    fn a_fabric_server_does_not_load_quilt_only_mods() {
        assert!(!version(&["quilt"]).is_compatible(&on("fabric")));
        assert!(version(&["fabric", "quilt"]).is_compatible(&on("fabric")));
    }
}
