//! Activation, valeurs, suppression et orphelins d'un contenu installé.

use super::*;

/// Active ou désactive un contenu installé (renommé en `.disabled` sur le nœud).
pub async fn toggle(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    content_id: Uuid,
    enabled: bool,
) -> Result<ContentRecord, ServersError> {
    let server = get(state, server_id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    let plan = plan_of(state, &server, slot).await?;
    let mut conn = state.pool.acquire().await?;
    let item = ContentRepository::get(&mut conn, server_id, content_id)
        .await?
        .filter(|c| c.slot == slot)
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))?;
    drop(conn);
    if item.modpack_id.is_some() || item.kind == "modpack" {
        return Err(ServersError::Conflict(ErrorCode::ContentInModpack));
    }
    if item.state != "installed" {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    // Référence, ou paquets extraits (ADR-0030) : le nœud n'a rien à renommer ; le serveur est
    // redéployé avec ses fichiers d'inscription à jour.
    let reference = item.kind == "reference" || plan.extract.is_some();
    let changed = item.enabled != enabled;
    let live = is_live(&server);
    if changed && !reference {
        let result = state
            .commands
            .send(
                NodeId::from(server.node_id),
                ApiMessage::ContentToggle(ContentToggle {
                    server_id,
                    service: plan.service.clone(),
                    dir: plan.dir.clone().unwrap_or_default(),
                    file_name: item.file_name.clone(),
                    enabled,
                }),
            )
            .await?;
        accepted(result)?;
    }
    // Le nœud a renommé le fichier : la base suit, sans connexion tenue pendant l'attente.
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if changed {
        if live {
            ServerRepository::defer_changes(&mut tx, server_id, &[content_id], 0).await?;
        }
        if reference && !live {
            ServerRepository::set_state(&mut tx, server_id, ServerState::Deploying.as_str(), None)
                .await?;
        }
        ContentRepository::set_enabled(&mut tx, content_id, enabled).await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentToggled)
            .target("server", server_id)
            .details(json!({ "slot": slot, "content_id": content_id, "name": item.name, "enabled": enabled })),
    )
    .await?;
    if reference && changed {
        apply_registered(state, server_id, live).await?;
    }
    let mut conn = state.pool.acquire().await?;
    ContentRepository::get(&mut conn, server_id, content_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))
}

/// Modifie les valeurs saisies d'un contenu, puis redéploie : les gabarits du serveur les
/// reprennent (ADR-0030).
pub async fn set_values(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    content_id: Uuid,
    values: &Map<String, Value>,
) -> Result<ContentRecord, ServersError> {
    let server = get(state, server_id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    let plan = plan_of(state, &server, slot).await?;
    let field_values = checked_values(&plan, values)?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    let item = ContentRepository::get(&mut tx, server_id, content_id)
        .await?
        .filter(|c| c.slot == slot)
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))?;
    if item.modpack_id.is_some() || item.kind == "modpack" {
        return Err(ServersError::Conflict(ErrorCode::ContentInModpack));
    }
    ContentRepository::set_field_values(&mut tx, content_id, &field_values).await?;
    let live = is_live(&server);
    if live {
        ServerRepository::defer_changes(&mut tx, server_id, &[content_id], 0).await?;
    } else {
        ServerRepository::set_state(&mut tx, server_id, ServerState::Deploying.as_str(), None)
            .await?;
    }
    let fields: Vec<&String> = field_values
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, _)| k)
        .collect();
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentValuesUpdated)
            .target("server", server_id)
            .details(json!({ "slot": slot, "content_id": content_id, "name": item.name, "fields": fields })),
    )
    .await?;
    apply_registered(state, server_id, live).await?;
    let mut conn = state.pool.acquire().await?;
    ContentRepository::get(&mut conn, server_id, content_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))
}

/// Dépendances que plus rien n'exigerait sans `removed`, récursivement.
pub fn orphans(removed: Uuid, links: &[(Uuid, Uuid)]) -> Vec<Uuid> {
    let mut gone: HashSet<Uuid> = HashSet::from([removed]);
    let mut changed = true;
    while changed {
        changed = false;
        let candidates: HashSet<Uuid> = links
            .iter()
            .filter(|(_, by)| gone.contains(by))
            .map(|(dep, _)| *dep)
            .filter(|dep| !gone.contains(dep))
            .collect();
        for dep in candidates {
            let still_required = links.iter().any(|(d, by)| *d == dep && !gone.contains(by));
            if !still_required {
                gone.insert(dep);
                changed = true;
            }
        }
    }
    gone.remove(&removed);
    let mut list: Vec<Uuid> = gone.into_iter().collect();
    list.sort();
    list
}

/// Supprime un contenu, et ses dépendances devenues inutiles avec `with_orphans`. Un contenu
/// encore exigé par un autre ne peut pas être supprimé seul.
pub async fn remove(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    content_id: Uuid,
    with_orphans: bool,
) -> Result<Vec<Uuid>, ServersError> {
    let server = get(state, server_id).await?.server;
    let plan = plan_of(state, &server, slot).await?;
    let mut conn = state.pool.acquire().await?;
    let items = ContentRepository::of_slot(&mut conn, server_id, slot).await?;
    let item = items
        .iter()
        .find(|c| c.id == content_id)
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))?;
    if item.modpack_id.is_some() {
        return Err(ServersError::Conflict(ErrorCode::ContentInModpack));
    }
    let links = ContentRepository::links(&mut conn, server_id).await?;
    if links
        .iter()
        .any(|(dep, by)| *dep == content_id && *by != content_id)
    {
        return Err(ServersError::Conflict(ErrorCode::ContentRequired));
    }
    let pack = item.kind == "modpack";
    let mut ids = vec![content_id];
    if pack {
        ids.extend(
            items
                .iter()
                .filter(|c| c.modpack_id == Some(content_id))
                .map(|c| c.id),
        );
    } else if with_orphans {
        ids.extend(orphans(content_id, &links));
    }
    let removed: Vec<&ContentRecord> = items.iter().filter(|c| ids.contains(&c.id)).collect();
    let extracted = plan.extract.is_some();
    // En mode `extract`, l'archive est partie : ce sont les dossiers de ses paquets.
    let packages: Vec<(Uuid, String)> = if extracted {
        ContentRepository::packages(&mut conn, server_id)
            .await?
            .into_iter()
            .filter(|(content_id, _)| ids.contains(content_id))
            .map(|(content_id, package)| (content_id, package.path()))
            .collect()
    } else {
        Vec::new()
    };
    drop(conn);
    let dirs: Vec<String> = packages.iter().map(|(_, dir)| dir.clone()).collect();
    // Les fichiers présents sur le nœud (installés, ou peut-être en cours) sont retirés ; un
    // contenu `extract` sans paquet (archive déposée telle quelle par un nœud plus ancien) aussi.
    let file_names: Vec<String> = removed
        .iter()
        .filter(|c| c.state != "failed" && c.kind == "file")
        .filter(|c| !extracted || !packages.iter().any(|(id, _)| *id == c.id))
        .map(|c| c.file_name.clone())
        .collect();
    // Les fichiers d'un modpack sont relatifs au dossier du jeu.
    let root = if pack { game_root(&plan) } else { None };
    if !file_names.is_empty() || !dirs.is_empty() {
        ensure_ready(&server)?;
        let result = state
            .commands
            .send(
                NodeId::from(server.node_id),
                ApiMessage::ContentRemove(ContentRemove {
                    server_id,
                    service: plan.service.clone(),
                    dir: plan.dir.clone().unwrap_or_default(),
                    root,
                    file_names,
                    dirs,
                }),
            )
            .await?;
        accepted(result)?;
    }
    // Les fichiers sont partis du nœud : la base suit, sans connexion tenue pendant l'attente.
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    // Une référence (ou un paquet inscrit) retirée : le jeu ne doit plus la recevoir.
    let references = extracted || removed.iter().any(|c| c.kind == "reference");
    let live = is_live(&server);
    if live {
        let count = i32::try_from(removed.len()).unwrap_or(i32::MAX);
        ServerRepository::defer_changes(&mut tx, server_id, &[], count).await?;
    } else if references {
        ServerRepository::set_state(&mut tx, server_id, ServerState::Deploying.as_str(), None)
            .await?;
    }
    ContentRepository::delete(&mut tx, &ids).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentRemoved)
            .target("server", server_id)
            .details(json!({
                "slot": slot,
                "name": item.name,
                "removed": removed.iter().map(|c| &c.name).collect::<Vec<_>>(),
            })),
    )
    .await?;
    if references {
        apply_registered(state, server_id, live).await?;
    }
    Ok(ids)
}
