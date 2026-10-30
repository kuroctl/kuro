//! Recherche, projets, catégories et versions chez les providers.

use super::*;

pub async fn search(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
    source: Option<&str>,
    text: &str,
    offset: u32,
    filters: &SearchFilters,
) -> Result<SearchPage, ServersError> {
    let categories = filters.validated()?;
    let server = get(state, server_id).await?.server;
    let (_, compat, provider) = usable_slot(state, &server, slot, source).await?;
    source_of(state, provider)?
        .search(&SearchQuery {
            text: text.chars().take(100).collect(),
            compat,
            offset: offset.min(10_000),
            limit: SEARCH_PAGE,
            sort: filters.sort,
            categories,
        })
        .await
        .map_err(provider_error)
}

/// Fiche d'un projet chez une source, avec les valeurs proposées pour les champs de
/// l'emplacement d'après sa description (ADR-0030).
pub async fn project(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
    source: Option<&str>,
    project_id: &str,
) -> Result<(crate::content::Project, Value), ServersError> {
    let server = get(state, server_id).await?.server;
    let (plan, _, provider) = usable_slot(state, &server, slot, source).await?;
    let project = source_of(state, provider)?
        .project(project_id)
        .await
        .map_err(provider_error)?;
    let values = prefilled(&plan, project.description.as_deref());
    Ok((project, values))
}

/// Catégories de la source pour cet emplacement (type de contenu, jeu), à proposer en filtres.
pub async fn categories(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
    source: Option<&str>,
) -> Result<Vec<Category>, ServersError> {
    let server = get(state, server_id).await?.server;
    let (_, compat, provider) = usable_slot(state, &server, slot, source).await?;
    source_of(state, provider)?
        .categories(&compat)
        .await
        .map_err(provider_error)
}

pub async fn versions(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
    source: Option<&str>,
    project_id: &str,
) -> Result<Vec<ProviderVersion>, ServersError> {
    let server = get(state, server_id).await?.server;
    let (_, compat, provider) = usable_slot(state, &server, slot, source).await?;
    source_of(state, provider)?
        .versions(project_id, &compat)
        .await
        .map_err(provider_error)
}
