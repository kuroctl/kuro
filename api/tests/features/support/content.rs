//! Sources de contenu simulées (Modrinth, CurseForge) : projets et versions déclarés par chaque
//! test.

use std::collections::HashMap;
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use kuro_api::content::{
    Category, Compat, ContentProvider, Dependency, InstalledRef, Loader, Project, ProjectHit,
    ProviderError, ProviderFile, ProviderVersion, SearchPage, SearchQuery,
};
use kuro_protocol::api::ContentHash;

/// Seule clé d'API que la source CurseForge simulée accepte.
pub const CURSEFORGE_KEY: &str = "good-curseforge-key";
/// Seule clé d'API que la source Steam Workshop simulée accepte.
pub const STEAM_KEY: &str = "good-steam-key";

/// Modrinth par défaut ; [`FakeSource::curseforge`] demande une clé d'API.
pub type FakeModrinth = FakeSource;

#[derive(Debug)]
pub struct FakeSource {
    projects: Mutex<Vec<Project>>,
    /// Dans l'ordre de publication : la dernière ajoutée est la plus récente.
    versions: Mutex<Vec<ProviderVersion>>,
    pub unavailable: Mutex<bool>,
    /// URL → contenu, pour `fetch` (archives de modpack).
    pub files: Mutex<HashMap<String, Vec<u8>>>,
    /// Préfixe des adresses de fichiers acceptées.
    download_prefix: &'static str,
    /// Source à clé d'API : la clé reçue, `None` tant qu'aucune n'est réglée.
    key: Option<Mutex<Option<String>>>,
    /// Clé que la source accepte.
    accepted_key: &'static str,
    /// Projet → description, pour le pré-remplissage des champs.
    pub descriptions: Mutex<HashMap<String, String>>,
    /// Dernière recherche reçue, filtres compris.
    pub last_search: Mutex<Option<SearchQuery>>,
    /// Catégories que la source propose.
    pub categories: Mutex<Vec<Category>>,
}

impl Default for FakeSource {
    fn default() -> Self {
        Self {
            projects: Mutex::default(),
            versions: Mutex::default(),
            unavailable: Mutex::default(),
            files: Mutex::default(),
            download_prefix: "https://cdn.modrinth.com/",
            key: None,
            accepted_key: "",
            descriptions: Mutex::default(),
            last_search: Mutex::default(),
            categories: Mutex::default(),
        }
    }
}

/// Version d'un projet pour `game_version` et `loader`, avec ses dépendances obligatoires.
pub fn version(
    id: &str,
    project: &str,
    game_version: &str,
    loader: &str,
    requires: &[&str],
) -> ProviderVersion {
    ProviderVersion {
        id: id.to_owned(),
        project_id: project.to_owned(),
        name: format!("{project} {id}"),
        version_number: id.to_owned(),
        published: "2026-10-01T00:00:00Z".to_owned(),
        loaders: vec![loader.to_owned()],
        game_versions: vec![game_version.to_owned()],
        files: vec![ProviderFile {
            url: format!(
                "https://cdn.modrinth.com/data/{project}/versions/{id}/{project}-{id}.jar"
            ),
            file_name: format!("{project}-{id}.jar"),
            hash: ContentHash::Sha512(sha(id)),
            size: 1024,
            primary: true,
        }],
        dependencies: requires
            .iter()
            .map(|project| Dependency {
                project_id: Some((*project).to_owned()),
                version_id: None,
                required: true,
            })
            .collect(),
    }
}

/// Empreinte factice, propre à chaque version.
pub fn sha(version_id: &str) -> String {
    let hex: String = version_id.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{hex:0<128}")
}

/// Fichier CurseForge : identifiants numériques, SHA-1, hébergé sur `edge.forgecdn.net`.
pub fn curseforge_version(
    file_id: u32,
    mod_id: u32,
    game_version: &str,
    loader: &str,
    requires: &[u32],
) -> ProviderVersion {
    let requires: Vec<String> = requires.iter().map(u32::to_string).collect();
    let requires: Vec<&str> = requires.iter().map(String::as_str).collect();
    let mut version = version(
        &file_id.to_string(),
        &mod_id.to_string(),
        game_version,
        loader,
        &requires,
    );
    let file = &mut version.files[0];
    file.url = format!("https://edge.forgecdn.net/files/{mod_id}/{file_id}/mod-{file_id}.jar");
    file.file_name = format!("mod-{file_id}.jar");
    let hex: String = file_id
        .to_string()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect();
    file.hash = ContentHash::Sha1(format!("{hex:0<40}"));
    version
}

impl FakeSource {
    /// CurseForge simulé : sans clé, la source n'est pas configurée.
    pub fn curseforge() -> Self {
        Self {
            download_prefix: "https://edge.forgecdn.net/",
            key: Some(Mutex::new(None)),
            accepted_key: CURSEFORGE_KEY,
            ..Self::default()
        }
    }

    /// Steam Workshop simulé : clé d'API, rien de téléchargeable par Kuro.
    pub fn steam() -> Self {
        Self {
            download_prefix: "https://never.invalid/",
            key: Some(Mutex::new(None)),
            accepted_key: STEAM_KEY,
            ..Self::default()
        }
    }

    pub fn add(&self, title: &str, version: ProviderVersion) {
        let mut projects = self.projects.lock().unwrap();
        if !projects.iter().any(|p| p.id == version.project_id) {
            projects.push(Project {
                id: version.project_id.clone(),
                title: title.to_owned(),
                icon_url: None,
                description: None,
                download_allowed: true,
            });
        }
        self.versions.lock().unwrap().push(version);
    }

    /// L'auteur du projet interdit le téléchargement par des tiers.
    pub fn forbid(&self, project_id: &str) {
        for project in self.projects.lock().unwrap().iter_mut() {
            if project.id == project_id {
                project.download_allowed = false;
            }
        }
    }

    /// Clé reçue par la source.
    pub fn key(&self) -> Option<String> {
        self.key.as_ref().and_then(|k| k.lock().unwrap().clone())
    }

    fn check(&self) -> Result<(), ProviderError> {
        if *self.unavailable.lock().unwrap() {
            return Err(ProviderError::Unavailable("down".to_owned()));
        }
        if !self.configured() {
            return Err(ProviderError::NotConfigured);
        }
        Ok(())
    }

    fn compatible(&self, project: &str, compat: &Compat) -> Vec<ProviderVersion> {
        let mut list: Vec<ProviderVersion> = self
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v.project_id == project && v.is_compatible(compat))
            .cloned()
            .collect();
        list.reverse();
        list
    }
}

impl ContentProvider for FakeSource {
    fn search(&self, query: &SearchQuery) -> BoxFuture<'_, Result<SearchPage, ProviderError>> {
        let query = query.clone();
        Box::pin(async move {
            self.check()?;
            *self.last_search.lock().unwrap() = Some(query.clone());
            let hits: Vec<ProjectHit> = self
                .projects
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.title.to_lowercase().contains(&query.text.to_lowercase()))
                .filter(|p| !self.compatible(&p.id, &query.compat).is_empty())
                .map(|p| ProjectHit {
                    page_url: Some(format!("https://example.test/projects/{}", p.id)),
                    download_allowed: p.download_allowed,
                    project_id: p.id.clone(),
                    slug: p.id.clone(),
                    title: p.title.clone(),
                    description: String::new(),
                    author: "someone".to_owned(),
                    icon_url: None,
                    downloads: 42,
                })
                .collect();
            Ok(SearchPage {
                total: hits.len() as u64,
                hits,
            })
        })
    }

    fn project(&self, project_id: &str) -> BoxFuture<'_, Result<Project, ProviderError>> {
        let id = project_id.to_owned();
        Box::pin(async move {
            self.check()?;
            self.projects
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .map(|mut project| {
                    project.description = self.descriptions.lock().unwrap().get(&id).cloned();
                    project
                })
                .ok_or(ProviderError::NotFound)
        })
    }

    fn versions(
        &self,
        project_id: &str,
        compat: &Compat,
    ) -> BoxFuture<'_, Result<Vec<ProviderVersion>, ProviderError>> {
        let (id, compat) = (project_id.to_owned(), compat.clone());
        Box::pin(async move {
            self.check()?;
            Ok(self.compatible(&id, &compat))
        })
    }

    fn version(&self, version_id: &str) -> BoxFuture<'_, Result<ProviderVersion, ProviderError>> {
        let id = version_id.to_owned();
        Box::pin(async move {
            self.check()?;
            self.versions
                .lock()
                .unwrap()
                .iter()
                .find(|v| v.id == id)
                .cloned()
                .ok_or(ProviderError::NotFound)
        })
    }

    fn latest(
        &self,
        installed: &[InstalledRef],
        compat: &Compat,
    ) -> BoxFuture<'_, Result<HashMap<String, ProviderVersion>, ProviderError>> {
        let (installed, compat) = (installed.to_vec(), compat.clone());
        Box::pin(async move {
            self.check()?;
            Ok(installed
                .into_iter()
                .filter_map(|item| {
                    let latest = self
                        .compatible(&item.project_id, &compat)
                        .into_iter()
                        .next()?;
                    Some((item.project_id, latest))
                })
                .collect())
        })
    }

    fn latest_game_version(&self) -> BoxFuture<'_, Result<String, ProviderError>> {
        Box::pin(async move { Ok("1.21.4".to_owned()) })
    }

    fn is_allowed_pack_download(&self, url: &str) -> bool {
        url.starts_with("https://cdn.modrinth.com/") || url.starts_with("https://github.com/")
    }

    fn fetch(&self, url: &str, _max_bytes: usize) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        let url = url.to_owned();
        Box::pin(async move {
            self.check()?;
            self.files
                .lock()
                .unwrap()
                .get(&url)
                .cloned()
                .ok_or(ProviderError::NotFound)
        })
    }

    fn is_allowed_download(&self, url: &str) -> bool {
        url.starts_with(self.download_prefix)
    }

    fn categories(&self, _compat: &Compat) -> BoxFuture<'_, Result<Vec<Category>, ProviderError>> {
        Box::pin(async move {
            self.check()?;
            Ok(self.categories.lock().unwrap().clone())
        })
    }

    fn configured(&self) -> bool {
        self.key
            .as_ref()
            .is_none_or(|key| key.lock().unwrap().is_some())
    }

    fn set_key(&self, key: Option<String>) {
        if let Some(slot) = &self.key {
            *slot.lock().unwrap() = key;
        }
    }

    fn verify_key(&self, key: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        let valid = key == self.accepted_key;
        Box::pin(async move {
            if *self.unavailable.lock().unwrap() {
                return Err(ProviderError::Unavailable("down".to_owned()));
            }
            if valid {
                Ok(())
            } else {
                Err(ProviderError::Unauthorized)
            }
        })
    }

    fn loaders(&self) -> BoxFuture<'_, Result<Vec<Loader>, ProviderError>> {
        Box::pin(async move {
            self.check()?;
            Ok(vec![
                Loader {
                    name: "fabric".to_owned(),
                    icon_svg: Some(r#"<svg viewBox="0 0 24 24"/>"#.to_owned()),
                },
                Loader {
                    name: "paper".to_owned(),
                    icon_svg: None,
                },
            ])
        })
    }
}
