//! Installation et mise à jour d'un contenu : plan, dépendances, envoi au nœud, références.

use super::*;

/// Contenu à déposer : version, nom du projet, icône, et contenu qui l'exige (dans le plan).
struct Planned {
    id: Uuid,
    version: ProviderVersion,
    name: String,
    icon_url: Option<String>,
    /// Description du projet, où chercher les valeurs proposées (ADR-0030).
    description: Option<String>,
    required_by: Vec<Uuid>,
}

/// Version demandée et ses dépendances obligatoires, résolues récursivement chez la même source.
/// Un projet déjà installé n'est pas repris : le lien est fait avec lui. Un projet dont l'auteur
/// interdit le téléchargement par des tiers est refusé.
/// `download` : le nœud téléchargera les fichiers ; en mode `reference`, c'est le jeu, et le
/// refus de distribution par des tiers ne s'applique pas.
async fn plan_install(
    provider: &dyn ContentProvider,
    source: &str,
    compat: &Compat,
    installed: &[ContentRecord],
    root: ProviderVersion,
    download: bool,
) -> Result<(Vec<Planned>, Vec<(Uuid, Uuid)>), ServersError> {
    let mut planned: Vec<Planned> = Vec::new();
    // Liens vers du contenu déjà installé : (contenu installé, contenu du plan qui l'exige).
    let mut existing_links = Vec::new();
    let mut by_project: HashMap<String, Uuid> = HashMap::new();
    let installed_by_project: HashMap<&str, Uuid> = installed
        .iter()
        .filter(|c| c.state != "failed" && c.source == source)
        .filter_map(|c| Some((c.project_id.as_deref()?, c.id)))
        .collect();

    let project = provider
        .project(&root.project_id)
        .await
        .map_err(provider_error)?;
    if download && !project.download_allowed {
        return Err(ServersError::Unprocessable(
            ErrorCode::ContentDistributionForbidden,
        ));
    }
    let root_id = Uuid::now_v7();
    by_project.insert(root.project_id.clone(), root_id);
    let mut queue = VecDeque::from([(root_id, root.dependencies.clone())]);
    planned.push(Planned {
        id: root_id,
        version: root,
        name: project.title,
        icon_url: project.icon_url,
        description: project.description,
        required_by: Vec::new(),
    });

    while let Some((parent, dependencies)) = queue.pop_front() {
        for dependency in dependencies.into_iter().filter(|d| d.required) {
            let version = match (&dependency.version_id, &dependency.project_id) {
                (Some(version_id), _) => {
                    provider.version(version_id).await.map_err(provider_error)?
                }
                (None, Some(project_id)) => {
                    if let Some(&id) = installed_by_project.get(project_id.as_str()) {
                        existing_links.push((id, parent));
                        continue;
                    }
                    if let Some(&id) = by_project.get(project_id) {
                        link(&mut planned, id, parent);
                        continue;
                    }
                    provider
                        .versions(project_id, compat)
                        .await
                        .map_err(provider_error)?
                        .into_iter()
                        .next()
                        .ok_or(ServersError::Unprocessable(ErrorCode::ContentIncompatible))?
                }
                (None, None) => continue,
            };
            if let Some(&id) = installed_by_project.get(version.project_id.as_str()) {
                existing_links.push((id, parent));
                continue;
            }
            if let Some(&id) = by_project.get(&version.project_id) {
                link(&mut planned, id, parent);
                continue;
            }
            if !version.is_compatible(compat) {
                return Err(ServersError::Unprocessable(ErrorCode::ContentIncompatible));
            }
            if planned.len() >= MAX_DEPENDENCIES {
                return Err(ServersError::Unprocessable(ErrorCode::ContentIncompatible));
            }
            let project = provider
                .project(&version.project_id)
                .await
                .map_err(provider_error)?;
            if download && !project.download_allowed {
                return Err(ServersError::Unprocessable(
                    ErrorCode::ContentDistributionForbidden,
                ));
            }
            let id = Uuid::now_v7();
            by_project.insert(version.project_id.clone(), id);
            queue.push_back((id, version.dependencies.clone()));
            planned.push(Planned {
                id,
                version,
                name: project.title,
                icon_url: project.icon_url,
                description: project.description,
                required_by: vec![parent],
            });
        }
    }
    Ok((planned, existing_links))
}

fn link(planned: &mut [Planned], id: Uuid, parent: Uuid) {
    if let Some(entry) = planned.iter_mut().find(|p| p.id == id)
        && !entry.required_by.contains(&parent)
    {
        entry.required_by.push(parent);
    }
}

/// Installe une version et ses dépendances obligatoires. Enregistré et journalisé avant l'envoi
/// (le nœud peut terminer avant notre réponse) ; un refus du nœud passe le contenu en échec.
/// En mode `reference`, seul l'identifiant est gardé, puis le serveur est redéployé.
pub async fn install(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    source: Option<&str>,
    version_id: &str,
    values: &Map<String, Value>,
) -> Result<Vec<ContentRecord>, ServersError> {
    let server = get(state, server_id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    ensure_below_quota(&server)?;
    let (plan, compat, kind) = usable_slot(state, &server, slot, source).await?;
    let field_values = checked_values(&plan, values)?;
    let provider = source_of(state, kind)?;
    let name = source_name(kind);
    let version = provider.version(version_id).await.map_err(provider_error)?;
    if !version.is_compatible(&compat) {
        return Err(ServersError::Unprocessable(ErrorCode::ContentIncompatible));
    }
    let mut conn = state.pool.acquire().await?;
    let current = ContentRepository::of_slot(&mut conn, server_id, slot).await?;
    drop(conn);
    if current.iter().any(|c| {
        c.state != "failed"
            && c.source == name
            && c.project_id.as_deref() == Some(version.project_id.as_str())
    }) {
        return Err(ServersError::Conflict(ErrorCode::ContentAlreadyInstalled));
    }
    if plan.dir.is_none() {
        return add_reference(
            state,
            context,
            &server,
            slot,
            provider.as_ref(),
            Chosen {
                source: name,
                version,
                values: &field_values,
                plan: &plan,
                compat: &compat,
                current: &current,
            },
        )
        .await;
    }
    let (planned, existing_links) =
        plan_install(provider.as_ref(), name, &compat, &current, version, true).await?;
    let operation_id = Uuid::now_v7();
    let files = files_of(provider.as_ref(), &planned)?;

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    // Une tentative en échec du même projet est remplacée.
    let retried: Vec<Uuid> = current
        .iter()
        .filter(|c| {
            c.state == "failed"
                && planned
                    .iter()
                    .any(|p| c.project_id.as_deref() == Some(p.version.project_id.as_str()))
        })
        .map(|c| c.id)
        .collect();
    ContentRepository::delete(&mut tx, &retried).await?;
    insert_planned(
        &mut tx,
        server_id,
        slot,
        name,
        operation_id,
        &planned,
        Root {
            replaces: None,
            values: &field_values,
        },
    )
    .await?;
    for (installed, required_by) in &existing_links {
        ContentRepository::link(&mut tx, *installed, *required_by).await?;
    }
    let root = &planned[0];
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentInstalled)
            .target("server", server_id)
            .details(json!({
                "slot": slot,
                "source": name,
                "project_id": root.version.project_id,
                "version_id": root.version.id,
                "dependencies": planned[1..].iter().map(|p| &p.version.project_id).collect::<Vec<_>>(),
            })),
    )
    .await?;

    send_install(
        state,
        context,
        &server,
        &plan,
        operation_id,
        files,
        &planned,
    )
    .await?;
    let ids: HashSet<Uuid> = planned.iter().map(|p| p.id).collect();
    let mut conn = state.pool.acquire().await?;
    Ok(ContentRepository::of_slot(&mut conn, server_id, slot)
        .await?
        .into_iter()
        .filter(|c| ids.contains(&c.id))
        .collect())
}

/// Met à jour vers la version compatible la plus récente ; l'ancienne est retirée quand la
/// nouvelle est installée.
pub async fn update(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    content_id: Uuid,
) -> Result<ContentRecord, ServersError> {
    let server = get(state, server_id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    ensure_below_quota(&server)?;
    let mut conn = state.pool.acquire().await?;
    let current = ContentRepository::of_slot(&mut conn, server_id, slot).await?;
    drop(conn);
    let item = current
        .iter()
        .find(|c| c.id == content_id)
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))?
        .clone();
    if item.modpack_id.is_some() || item.kind == "modpack" {
        return Err(ServersError::Conflict(ErrorCode::ContentInModpack));
    }
    let (Some(project_id), "modrinth" | "curseforge", "installed", "file") = (
        item.project_id.clone(),
        item.source.as_str(),
        item.state.as_str(),
        item.kind.as_str(),
    ) else {
        return Err(ServersError::Conflict(ErrorCode::ContentUpToDate));
    };
    let (plan, compat, kind) = usable_slot(state, &server, slot, Some(&item.source)).await?;
    let provider = source_of(state, kind)?;
    let name = source_name(kind);
    let latest = provider
        .versions(&project_id, &compat)
        .await
        .map_err(provider_error)?
        .into_iter()
        .next()
        .ok_or(ServersError::Unprocessable(ErrorCode::ContentIncompatible))?;
    if Some(&latest.id) == item.version_id.as_ref() {
        return Err(ServersError::Conflict(ErrorCode::ContentUpToDate));
    }
    let others: Vec<ContentRecord> = current.into_iter().filter(|c| c.id != content_id).collect();
    let (planned, existing_links) =
        plan_install(provider.as_ref(), name, &compat, &others, latest, true).await?;
    let operation_id = Uuid::now_v7();
    let files = files_of(provider.as_ref(), &planned)?;

    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    insert_planned(
        &mut tx,
        server_id,
        slot,
        name,
        operation_id,
        &planned,
        Root {
            replaces: Some(content_id),
            values: &item.field_values,
        },
    )
    .await?;
    for (installed, required_by) in &existing_links {
        ContentRepository::link(&mut tx, *installed, *required_by).await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentUpdated)
            .target("server", server_id)
            .details(json!({
                "slot": slot,
                "project_id": project_id,
                "from": item.version_id,
                "to": planned[0].version.id,
            })),
    )
    .await?;

    send_install(
        state,
        context,
        &server,
        &plan,
        operation_id,
        files,
        &planned,
    )
    .await?;
    let mut conn = state.pool.acquire().await?;
    ContentRepository::get(&mut conn, server_id, planned[0].id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))
}

fn files_of(
    provider: &dyn ContentProvider,
    planned: &[Planned],
) -> Result<Vec<ContentFile>, ServersError> {
    planned
        .iter()
        .map(|p| {
            // Sans fichier : l'auteur refuse la distribution par des tiers (CurseForge).
            let file = p.version.primary_file().ok_or(ServersError::Unprocessable(
                ErrorCode::ContentDistributionForbidden,
            ))?;
            if !provider.is_allowed_download(&file.url) {
                return Err(ServersError::Unprocessable(ErrorCode::ContentIncompatible));
            }
            Ok(ContentFile {
                url: file.url.clone(),
                hash: file.hash.clone(),
                size: file.size,
                file_name: file.file_name.clone(),
            })
        })
        .collect()
}

/// Contenu demandé par l'admin, en tête du plan : la version qu'il remplace (mise à jour) et les
/// valeurs saisies pour lui ; ses dépendances n'ont ni l'un ni l'autre.
struct Root<'a> {
    replaces: Option<Uuid>,
    values: &'a Value,
}

async fn insert_planned(
    conn: &mut sqlx::PgConnection,
    server_id: Uuid,
    slot: &str,
    source: &str,
    operation_id: Uuid,
    planned: &[Planned],
    root: Root<'_>,
) -> Result<(), ServersError> {
    let Root {
        replaces,
        values: root_values,
    } = root;
    let none = json!({});
    for (index, entry) in planned.iter().enumerate() {
        let file = entry
            .version
            .primary_file()
            .ok_or(ServersError::Unprocessable(
                ErrorCode::ContentDistributionForbidden,
            ))?;
        let (sha512, sha1) = match &file.hash {
            ContentHash::Sha512(value) => (Some(value.as_str()), None),
            ContentHash::Sha1(value) => (None, Some(value.as_str())),
        };
        ContentRepository::insert(
            conn,
            NewContent {
                id: entry.id,
                server_id,
                slot,
                source,
                project_id: Some(&entry.version.project_id),
                version_id: Some(&entry.version.id),
                name: &entry.name,
                version_name: Some(&entry.version.version_number),
                icon_url: entry.icon_url.as_deref(),
                file_name: &file.file_name,
                sha512,
                sha1,
                // Les valeurs saisies vont au contenu demandé, pas à ses dépendances.
                field_values: if index == 0 { root_values } else { &none },
                size_bytes: i64::try_from(file.size).ok(),
                operation_id: Some(operation_id),
                replaces: if index == 0 { replaces } else { None },
                kind: "file",
                modpack_id: None,
                state: "installing",
            },
        )
        .await?;
    }
    for entry in planned {
        for parent in &entry.required_by {
            ContentRepository::link(conn, entry.id, *parent).await?;
        }
    }
    Ok(())
}

async fn send_install(
    state: &ServersState,
    context: &AuditContext,
    server: &ServerRecord,
    plan: &ContentPlan,
    operation_id: Uuid,
    files: Vec<ContentFile>,
    planned: &[Planned],
) -> Result<(), ServersError> {
    let dir = plan
        .dir
        .clone()
        .ok_or(ServersError::Unprocessable(ErrorCode::ContentNotSupported))?;
    let sent = state
        .commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::ContentInstall(ContentInstall {
                server_id: server.id,
                operation_id,
                service: plan.service.clone(),
                dir,
                files,
                root: None,
                archive: None,
                extract: plan.extract.clone(),
            }),
        )
        .await
        .map_err(ServersError::from)
        .and_then(accepted);
    if let Err(error) = sent {
        let ids: Vec<Uuid> = planned.iter().map(|p| p.id).collect();
        let mut tx = AuditedTx::begin(&state.pool, context).await?;
        ContentRepository::fail(&mut tx, &ids, refusal(&error)).await?;
        tx.commit(
            AuditEvent::failure(AuditAction::ServerContentInstalled)
                .target("server", server.id)
                .details(json!({ "slot": plan.slot, "reason": refusal(&error) })),
        )
        .await?;
        return Err(error);
    }
    Ok(())
}

/// Mode `reference` (ARK) : le jeu télécharge lui-même ses mods. Kuro garde l'identifiant, puis
/// redéploie le serveur pour qu'il le reçoive (`{{ content.<slot>.ids }}`).
/// Version choisie chez une source, avec les valeurs saisies pour elle, et ce qu'il faut pour
/// résoudre ses dépendances.
struct Chosen<'a> {
    source: &'a str,
    version: ProviderVersion,
    values: &'a Value,
    plan: &'a ContentPlan,
    compat: &'a Compat,
    current: &'a [ContentRecord],
}

/// Valeurs proposées d'après la description d'un contenu (`prefill` des champs, ADR-0030).
pub(super) fn prefilled(plan: &ContentPlan, description: Option<&str>) -> Value {
    let description = description.unwrap_or_default();
    let mut out = Map::new();
    for (key, field) in &plan.fields {
        let Some(prefill) = &field.prefill else {
            continue;
        };
        let mut values = prefill.values(description);
        if field.kind == ContentFieldType::String {
            values.truncate(1);
        }
        if !values.is_empty() {
            out.insert(key.clone(), json!(values));
        }
    }
    Value::Object(out)
}

/// Mode `reference` (ARK, Project Zomboid) : le jeu télécharge lui-même ses mods. Kuro garde les
/// identifiants du contenu demandé et de ses dépendances (chacune avec les valeurs proposées par
/// sa description), puis redéploie le serveur pour qu'il les reçoive (`{{ content.<slot>.ids }}`).
async fn add_reference(
    state: &ServersState,
    context: &AuditContext,
    server: &ServerRecord,
    slot: &str,
    provider: &dyn ContentProvider,
    chosen: Chosen<'_>,
) -> Result<Vec<ContentRecord>, ServersError> {
    let Chosen {
        source,
        version,
        values: field_values,
        plan,
        compat,
        current,
    } = chosen;
    let (planned, existing_links) =
        plan_install(provider, source, compat, current, version, false).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    for (index, entry) in planned.iter().enumerate() {
        let values = if index == 0 {
            field_values.clone()
        } else {
            prefilled(plan, entry.description.as_deref())
        };
        ContentRepository::insert(
            &mut tx,
            NewContent {
                id: entry.id,
                server_id: server.id,
                slot,
                source,
                project_id: Some(&entry.version.project_id),
                version_id: Some(&entry.version.id),
                name: &entry.name,
                version_name: Some(&entry.version.version_number),
                icon_url: entry.icon_url.as_deref(),
                // Aucun fichier : l'identifiant en tient lieu.
                file_name: &entry.version.project_id,
                sha512: None,
                sha1: None,
                field_values: &values,
                size_bytes: None,
                operation_id: None,
                replaces: None,
                kind: "reference",
                modpack_id: None,
                state: "installed",
            },
        )
        .await?;
    }
    for entry in &planned {
        for parent in &entry.required_by {
            ContentRepository::link(&mut tx, entry.id, *parent).await?;
        }
    }
    for (installed, required_by) in &existing_links {
        ContentRepository::link(&mut tx, *installed, *required_by).await?;
    }
    let live = is_live(server);
    if live {
        let ids: Vec<Uuid> = planned.iter().map(|p| p.id).collect();
        ServerRepository::defer_changes(&mut tx, server.id, &ids, 0).await?;
    } else {
        ServerRepository::set_state(&mut tx, server.id, ServerState::Deploying.as_str(), None)
            .await?;
    }
    let root = &planned[0];
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentInstalled)
            .target("server", server.id)
            .details(json!({
                "slot": slot,
                "source": source,
                "project_id": root.version.project_id,
                "version_id": root.version.id,
                "dependencies": planned[1..].iter().map(|p| &p.version.project_id).collect::<Vec<_>>(),
                "mode": "reference",
            })),
    )
    .await?;
    apply_registered(state, server.id, live).await?;
    let ids: HashSet<Uuid> = planned.iter().map(|p| p.id).collect();
    let mut conn = state.pool.acquire().await?;
    Ok(ContentRepository::of_slot(&mut conn, server.id, slot)
        .await?
        .into_iter()
        .filter(|c| ids.contains(&c.id))
        .collect())
}
