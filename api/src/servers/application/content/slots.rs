//! Emplacements du prefab, leurs sources et ce qui y est installé.

use super::*;

/// Loaders de la source de contenu et leurs icônes (Modrinth les décrit).
pub async fn loaders(state: &ServersState) -> Result<Vec<Loader>, ServersError> {
    state
        .content
        .modrinth
        .loaders()
        .await
        .map_err(provider_error)
}

/// Emplacements du serveur.
pub async fn slots(state: &ServersState, server_id: Uuid) -> Result<Vec<Slot>, ServersError> {
    let server = get(state, server_id).await?.server;
    let plans = content_plans(state, &server).await?;
    let mut slots = Vec::with_capacity(plans.len());
    for plan in plans {
        slots.push(slot_of(state, plan).await);
    }
    Ok(slots)
}

async fn slot_of(state: &ServersState, plan: ContentPlan) -> Slot {
    let mut sources = Vec::new();
    let mut unpinned = false;
    for decl in &plan.providers {
        let Some(provider) = state.content.get(decl.provider) else {
            // Source déclarée que Kuro ne gère pas encore (Thunderstore, uMod à venir) : signalée
            // telle quelle.
            if decl.provider != Provider::Upload {
                sources.push(SlotSource {
                    provider: decl.provider,
                    compat: None,
                    blocked: Some(ErrorCode::ContentSourceUnsupported),
                });
            }
            continue;
        };
        let blocked = source_blocked(&plan, decl, provider.configured());
        unpinned |= blocked == Some(ErrorCode::GameVersionNotPinned);
        let compat = blocked.is_none().then(|| Compat {
            project_type: project_type(plan.kind).to_owned(),
            game: decl.game.clone(),
            class: decl.class.clone(),
            loader: decl.loader.clone().filter(|l| !l.is_empty()),
            game_version: decl.game_version.clone().unwrap_or_default(),
        });
        sources.push(SlotSource {
            provider: decl.provider,
            compat,
            blocked,
        });
    }
    let latest_game_version = if unpinned {
        state.content.modrinth.latest_game_version().await.ok()
    } else {
        None
    };
    Slot {
        plan,
        sources,
        latest_game_version,
    }
}

/// Envoi de fichiers déclaré par le prefab (`provider: upload`), sur un emplacement qui a un
/// dossier.
pub(super) fn accepts_upload(plan: &ContentPlan) -> bool {
    // Mode `extract` : une archive envoyée n'est pas encore extraite par le nœud.
    plan.dir.is_some()
        && plan.extract.is_none()
        && plan
            .providers
            .iter()
            .any(|p| p.provider == Provider::Upload)
}

/// Ce qui empêche une source de l'emplacement d'être utilisée.
fn source_blocked(plan: &ContentPlan, decl: &ContentSource, configured: bool) -> Option<ErrorCode> {
    // Un serveur vanilla ne charge aucun mod : loader déclaré, mais vide ou `vanilla`.
    if plan.kind == ContentKind::Mods
        && decl
            .loader
            .as_deref()
            .is_some_and(|loader| loader.is_empty() || loader == "vanilla")
    {
        return Some(ErrorCode::ContentNotSupported);
    }
    // Une version du jeu déclarée doit être figée : la suivante casserait le contenu. Sans
    // version déclarée (ARK), tout fichier convient.
    if decl.game_version.as_deref().is_some_and(|v| !is_pinned(v)) {
        return Some(ErrorCode::GameVersionNotPinned);
    }
    if !configured {
        return Some(ErrorCode::ContentSourceNotConfigured);
    }
    None
}

pub(super) fn source_of(
    state: &ServersState,
    provider: Provider,
) -> Result<&std::sync::Arc<dyn ContentProvider>, ServersError> {
    state
        .content
        .get(provider)
        .ok_or(ServersError::Unprocessable(ErrorCode::ContentNotSupported))
}

/// Emplacement et source utilisables : la source demandée (`modrinth`, `curseforge`), ou celle
/// proposée par défaut.
pub(super) async fn usable_slot(
    state: &ServersState,
    server: &ServerRecord,
    slot: &str,
    source: Option<&str>,
) -> Result<(ContentPlan, Compat, Provider), ServersError> {
    let plan = content_plans(state, server)
        .await?
        .into_iter()
        .find(|plan| plan.slot == slot)
        .ok_or(ServersError::Missing(ErrorCode::ContentSlotNotFound))?;
    let slot = slot_of(state, plan).await;
    let chosen = match source {
        Some(name) => slot
            .sources
            .iter()
            .find(|candidate| source_name(candidate.provider) == name),
        None => slot.default_source(),
    }
    .ok_or(ServersError::Unprocessable(ErrorCode::ContentNotSupported))?;
    match (&chosen.compat, chosen.blocked) {
        (Some(compat), None) => Ok((slot.plan.clone(), compat.clone(), chosen.provider)),
        (_, Some(code)) => Err(ServersError::Unprocessable(code)),
        (None, None) => Err(ServersError::Unprocessable(ErrorCode::ContentNotSupported)),
    }
}

/// Contenu installé de l'emplacement, avec les mises à jour connues du provider. Un provider
/// injoignable n'empêche pas la liste : les mises à jour manquent seulement.
pub async fn installed(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
) -> Result<Installed, ServersError> {
    let server = get(state, server_id).await?.server;
    let plan = content_plans(state, &server)
        .await?
        .into_iter()
        .find(|plan| plan.slot == slot)
        .ok_or(ServersError::Missing(ErrorCode::ContentSlotNotFound))?;
    let mut conn = state.pool.acquire().await?;
    let items = ContentRepository::of_slot(&mut conn, server_id, slot).await?;
    let links = ContentRepository::links(&mut conn, server_id).await?;
    drop(conn);

    let mut updates = HashMap::new();
    for source in slot_of(state, plan).await.sources {
        let (Some(compat), Some(provider)) = (&source.compat, state.content.get(source.provider))
        else {
            continue;
        };
        let name = source_name(source.provider);
        let mine: Vec<&ContentRecord> = items
            .iter()
            .filter(|c| c.source == name && c.state == "installed" && c.kind == "file")
            .collect();
        let refs: Vec<InstalledRef> = mine
            .iter()
            .filter_map(|c| {
                Some(InstalledRef {
                    project_id: c.project_id.clone()?,
                    sha512: c.sha512.clone(),
                })
            })
            .collect();
        if refs.is_empty() {
            continue;
        }
        match provider.latest(&refs, compat).await {
            Ok(latest) => {
                for item in mine {
                    if let Some(version) = item.project_id.as_ref().and_then(|p| latest.get(p))
                        && Some(&version.id) != item.version_id.as_ref()
                    {
                        updates.insert(item.id, version.clone());
                    }
                }
            }
            Err(error) => tracing::warn!(%error, source = name, "content updates unavailable"),
        }
    }
    let mut packages: HashMap<Uuid, Vec<crate::servers::infra::ContentPackage>> = HashMap::new();
    {
        let mut conn = state.pool.acquire().await?;
        for (content_id, package) in ContentRepository::packages(&mut conn, server_id).await? {
            packages.entry(content_id).or_default().push(package);
        }
    }
    let pending: HashSet<Uuid> = {
        let mut conn = state.pool.acquire().await?;
        ContentRepository::pending_ids(&mut conn, server_id)
            .await?
            .into_iter()
            .collect()
    };
    Ok(Installed {
        items,
        links,
        updates,
        packages,
        pending,
    })
}
