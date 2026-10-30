//! Providers de contenu (ADR-0020, ADR-0029) : sources de mods, plugins et modpacks, derrière
//! une interface. Modrinth est ouvert ; CurseForge et Steam Workshop demandent la clé d'API de
//! l'installation, réglée dans le panel. Les cas d'usage (installer, mettre à jour…) sont dans le domaine
//! `servers`.

mod client;
pub mod curseforge;
pub mod http;
pub mod modrinth;
mod provider;
pub mod settings;
mod sources;
pub mod steam;
pub mod steam_store;
pub mod steamgriddb;

pub use provider::{
    Category, Compat, ContentProvider, Dependency, InstalledRef, Loader, Project, ProjectHit,
    ProviderError, ProviderFile, ProviderVersion, SearchPage, SearchQuery, SearchSort,
    accepted_loaders,
};
pub use sources::{ContentSources, project_page, source_name};
