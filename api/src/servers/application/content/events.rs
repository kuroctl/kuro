//! Retours du nœud : fin d'installation, redéploiement, retrait des versions remplacées.

use super::*;

/// Fin d'une installation rapportée par le nœud. Une mise à jour réussie retire l'ancienne
/// version (son fichier, s'il porte un autre nom, et sa ligne), en gardant ses liens. Des
/// paquets extraits (ADR-0030) sont enregistrés : le serveur est alors à redéployer, pour ses
/// fichiers d'inscription (`Some(serveur)`).
pub async fn record_done(
    pool: &PgPool,
    commands: &dyn NodeCommands,
    node: NodeId,
    done: ContentDone,
) -> Result<Option<Uuid>, sqlx::Error> {
    let failure = match &done.outcome {
        ContentOutcome::Ok => None,
        ContentOutcome::Failed { reason } => Some(reason.as_str()),
    };
    let mut tx = pool.begin().await?;
    let finished = ContentRepository::complete(
        &mut tx,
        node.as_uuid(),
        done.server_id,
        done.operation_id,
        failure,
    )
    .await?;
    // Installé pendant que le serveur tourne : chargé au prochain démarrage.
    let live = crate::servers::infra::ServerRepository::get(&mut tx, done.server_id)
        .await?
        .is_some_and(|server| is_live(&server));
    if live {
        let installed: Vec<Uuid> = finished
            .iter()
            .filter(|c| c.state == "installed")
            .map(|c| c.id)
            .collect();
        if !installed.is_empty() {
            ServerRepository::defer_changes(&mut tx, done.server_id, &installed, 0).await?;
        }
    }
    // Paquets extraits : rattachés au contenu dont l'archive porte ce nom.
    let mut registered = false;
    for item in finished.iter().filter(|c| c.state == "installed") {
        let packages: Vec<crate::servers::infra::ContentPackage> = done
            .packages
            .iter()
            .filter(|p| p.file_name == item.file_name)
            .map(|p| crate::servers::infra::ContentPackage {
                dir: p.dir.clone(),
                name: p.name.clone(),
                description: p.description.clone(),
            })
            .collect();
        if !packages.is_empty() {
            ContentRepository::set_packages(&mut tx, item.id, &packages).await?;
            registered = true;
        }
    }
    let mut stale = Vec::new();
    for new in finished.iter().filter(|c| c.state == "installed") {
        let Some(old_id) = new.replaces else { continue };
        let Some(old) = ContentRepository::get(&mut tx, done.server_id, old_id).await? else {
            continue;
        };
        // Paquets de l'ancienne version que la nouvelle n'a pas remplacés (autre nom d'archive).
        let kept: HashSet<String> = done
            .packages
            .iter()
            .filter(|p| p.file_name == new.file_name)
            .map(|p| format!("{}/{}", p.dir.trim_end_matches('/'), p.name))
            .collect();
        let old_dirs: Vec<String> = ContentRepository::packages(&mut tx, done.server_id)
            .await?
            .into_iter()
            .filter(|(content_id, _)| *content_id == old.id)
            .map(|(_, package)| package.path())
            .collect();
        ContentRepository::move_links(&mut tx, old.id, new.id).await?;
        ContentRepository::delete(&mut tx, &[old.id]).await?;
        let dirs: Vec<String> = old_dirs
            .iter()
            .filter(|dir| !kept.contains(*dir))
            .cloned()
            .collect();
        // Une archive extraite n'existe plus sur le nœud : seuls ses dossiers sont à retirer.
        let files = if old.file_name != new.file_name && old_dirs.is_empty() {
            vec![old.file_name.clone()]
        } else {
            Vec::new()
        };
        if !files.is_empty() || !dirs.is_empty() {
            stale.push((old.slot.clone(), files, dirs));
        }
    }
    tx.commit().await?;

    for (slot, files, dirs) in stale {
        if let Err(error) =
            remove_stale(pool, commands, node, done.server_id, &slot, files, dirs).await
        {
            tracing::warn!(server_id = %done.server_id, %error, "old content version not removed");
        }
    }
    Ok(registered.then_some(done.server_id))
}

/// Redéploie un serveur dont les paquets inscrits ont changé, sans redémarrer un serveur en
/// marche ; un échec passe le serveur en `failed`, avec sa raison.
pub async fn redeploy_registered(state: &ServersState, server_id: Uuid) {
    let live = match get(state, server_id).await {
        Ok(view) => is_live(&view.server),
        // Supprimé entre-temps : plus rien à redéployer.
        Err(ServersError::NotFound) => return,
        Err(error) => {
            tracing::warn!(server_id = %server_id, %error, "content redeploy skipped");
            return;
        }
    };
    // L'échec est porté par le serveur (`failed`, avec sa raison) ; la trace est pour l'exploitant.
    if let Err(error) = apply_registered(state, server_id, live).await {
        tracing::warn!(server_id = %server_id, %error, "content redeploy failed");
    }
}

/// Fichier (ou dossiers de paquets) d'une version remplacée : retiré du nœud, au mieux.
async fn remove_stale(
    pool: &PgPool,
    commands: &dyn NodeCommands,
    node: NodeId,
    server_id: Uuid,
    slot: &str,
    file_names: Vec<String>,
    dirs: Vec<String>,
) -> Result<(), String> {
    let mut conn = pool.acquire().await.map_err(|e| e.to_string())?;
    let server = crate::servers::infra::ServerRepository::get(&mut conn, server_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("server gone")?;
    drop(conn);
    let package =
        crate::prefabs::application::package(pool, &server.prefab_id, &server.prefab_version)
            .await
            .map_err(|e| e.to_string())?;
    let slot_def = package.manifest.content.get(slot).ok_or("slot gone")?;
    // Mode `extract` : les archives arrivent dans le dossier de la première route.
    let dir = slot_def
        .install
        .dir
        .clone()
        .or_else(|| slot_def.install.route.first().map(|r| r.dir.clone()))
        .ok_or("no dir")?;
    let result = commands
        .send(
            node,
            ApiMessage::ContentRemove(ContentRemove {
                server_id,
                service: slot_def.service.clone(),
                dir,
                root: None,
                file_names,
                dirs,
            }),
        )
        .await
        .map_err(|e| e.to_string())?;
    match result {
        kuro_protocol::daemon::server::CommandResult::Ok => Ok(()),
        other => Err(format!("{other:?}")),
    }
}
