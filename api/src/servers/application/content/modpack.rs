//! Modpacks (ADR-0020) : recherche, lecture de l'index, installation fichier par fichier.

use super::*;

/// Archive de modpack téléchargée au plus.
const MAX_PACK_BYTES: usize = 128 * 1024 * 1024;
/// Index d'un modpack lu au plus.
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
/// Fichiers d'un modpack au plus.
const MAX_PACK_FILES: usize = 2000;
/// Un modpack attend la fin du redéploiement (version, loader) au plus pendant ce temps.
const PACK_RETRIES: usize = 150;
const PACK_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Dossier du jeu : parent du dossier de l'emplacement (`/data/mods` → `/data`).
pub(super) fn game_root(plan: &ContentPlan) -> Option<String> {
    let dir = plan.dir.as_deref()?.trim_end_matches('/');
    let (parent, _) = dir.rsplit_once('/')?;
    (!parent.is_empty()).then(|| parent.to_owned())
}

/// Réglage visé par un gabarit simple (`{{ settings.version }}` → `version`).
fn setting_key(template: &str) -> Option<&str> {
    let inner = template
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim();
    let key = inner.strip_prefix("settings.")?;
    key.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        .then_some(key)
}

/// Emplacement de mods Minecraft, son dossier du jeu, et les réglages du loader et de la
/// version du jeu (ce qu'un modpack impose).
async fn pack_slot(
    state: &ServersState,
    server: &ServerRecord,
    slot: &str,
) -> Result<(ContentPlan, String, String, String), ServersError> {
    let plan = plan_of(state, server, slot).await?;
    let package = crate::prefabs::application::package(
        &state.pool,
        &server.prefab_id,
        &server.prefab_version,
    )
    .await?;
    let decl = package
        .manifest
        .content
        .get(slot)
        .and_then(|def| {
            def.providers
                .iter()
                .find(|p| p.provider == Provider::Modrinth)
        })
        .filter(|p| p.game.as_deref() == Some("minecraft"))
        .ok_or(ServersError::Unprocessable(ErrorCode::ContentNotSupported))?;
    let loader = decl.loader.as_deref().and_then(setting_key);
    let version = decl.game_version.as_deref().and_then(setting_key);
    match (plan.kind, game_root(&plan), loader, version) {
        (ContentKind::Mods, Some(root), Some(loader), Some(version)) => {
            Ok((plan.clone(), root, loader.to_owned(), version.to_owned()))
        }
        _ => Err(ServersError::Unprocessable(ErrorCode::ContentNotSupported)),
    }
}

fn pack_compat() -> Compat {
    Compat {
        project_type: "modpack".to_owned(),
        game: Some("minecraft".to_owned()),
        class: None,
        loader: None,
        game_version: String::new(),
    }
}

pub async fn modpack_search(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
    text: &str,
    offset: u32,
) -> Result<SearchPage, ServersError> {
    let server = get(state, server_id).await?.server;
    pack_slot(state, &server, slot).await?;
    state
        .content
        .modrinth
        .search(&SearchQuery {
            text: text.chars().take(100).collect(),
            compat: pack_compat(),
            offset: offset.min(10_000),
            limit: SEARCH_PAGE,
            sort: SearchSort::Relevance,
            categories: Vec::new(),
        })
        .await
        .map_err(provider_error)
}

pub async fn modpack_versions(
    state: &ServersState,
    server_id: Uuid,
    slot: &str,
    project_id: &str,
) -> Result<Vec<ProviderVersion>, ServersError> {
    let server = get(state, server_id).await?.server;
    pack_slot(state, &server, slot).await?;
    state
        .content
        .modrinth
        .versions(project_id, &pack_compat())
        .await
        .map_err(provider_error)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackIndex {
    game: String,
    #[serde(default)]
    files: Vec<PackFile>,
    #[serde(default)]
    dependencies: HashMap<String, String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    path: String,
    #[serde(default)]
    hashes: HashMap<String, String>,
    #[serde(default)]
    env: Option<HashMap<String, String>>,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    file_size: u64,
}

fn invalid_pack() -> ServersError {
    ServersError::Unprocessable(ErrorCode::InvalidModpack)
}

/// `modrinth.index.json` d'une archive `.mrpack`.
fn read_index(archive: &[u8]) -> Result<PackIndex, ServersError> {
    use std::io::Read;
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|_| invalid_pack())?;
    let entry = zip
        .by_name("modrinth.index.json")
        .map_err(|_| invalid_pack())?;
    let mut text = String::new();
    entry
        .take(MAX_INDEX_BYTES)
        .read_to_string(&mut text)
        .map_err(|_| invalid_pack())?;
    serde_json::from_str(&text).map_err(|_| invalid_pack())
}

/// Loader de Kuro pour la dépendance d'un modpack.
fn pack_loader(dependencies: &HashMap<String, String>) -> Option<&'static str> {
    [
        ("fabric-loader", "fabric"),
        ("quilt-loader", "quilt"),
        ("forge", "forge"),
        ("neoforge", "neoforge"),
    ]
    .into_iter()
    .find(|(dependency, _)| dependencies.contains_key(*dependency))
    .map(|(_, loader)| loader)
}

/// Chemin d'un fichier d'index : relatif, sans `..`.
fn pack_path(path: &str) -> Option<String> {
    if path.starts_with('/') || path.contains('\\') {
        return None;
    }
    let parts = kuro_protocol::limits::path_segments(path)?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Installe un modpack Modrinth : il impose la version du jeu et le loader (redéploiement s'ils
/// changent), ses fichiers côté serveur sont téléchargés et vérifiés, ses `overrides` extraits
/// dans le dossier du jeu. Un seul modpack par emplacement.
pub async fn install_modpack(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    version_id: &str,
) -> Result<Vec<ContentRecord>, ServersError> {
    use sha2::Digest;
    let server = get(state, server_id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    ensure_below_quota(&server)?;
    let (plan, root, loader_key, version_key) = pack_slot(state, &server, slot).await?;
    let mut conn = state.pool.acquire().await?;
    let current = ContentRepository::of_slot(&mut conn, server_id, slot).await?;
    drop(conn);
    if current.iter().any(|c| c.kind == "modpack") {
        return Err(ServersError::Conflict(ErrorCode::ModpackInstalled));
    }

    let modrinth = &state.content.modrinth;
    let version = modrinth.version(version_id).await.map_err(provider_error)?;
    let project = modrinth
        .project(&version.project_id)
        .await
        .map_err(provider_error)?;
    let pack = version.primary_file().cloned().ok_or_else(invalid_pack)?;
    if !modrinth.is_allowed_download(&pack.url) {
        return Err(invalid_pack());
    }
    let ContentHash::Sha512(pack_sha512) = pack.hash.clone() else {
        return Err(invalid_pack());
    };
    let bytes = modrinth
        .fetch(&pack.url, MAX_PACK_BYTES)
        .await
        .map_err(provider_error)?;
    // Empreinte et lecture de l'archive (jusqu'à 128 Mio) hors du runtime : c'est du calcul.
    let (digest, index) = tokio::task::spawn_blocking(move || {
        let digest = kuro_runtime::tls::hex(&sha2::Sha512::digest(&bytes));
        (digest, read_index(&bytes))
    })
    .await
    .map_err(|e| ServersError::Internal(e.to_string()))?;
    if !digest.eq_ignore_ascii_case(&pack_sha512) {
        return Err(invalid_pack());
    }
    let index = index?;
    let game_version = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(invalid_pack)?;
    if index.game != "minecraft" {
        return Err(invalid_pack());
    }
    let loader = pack_loader(&index.dependencies)
        .ok_or(ServersError::Unprocessable(ErrorCode::ContentNotSupported))?;

    let mut files = Vec::new();
    for file in index.files.iter().filter(|f| {
        f.env
            .as_ref()
            .and_then(|env| env.get("server"))
            .is_none_or(|server| server != "unsupported")
    }) {
        let path = pack_path(&file.path).ok_or_else(invalid_pack)?;
        let sha512 = file
            .hashes
            .get("sha512")
            .cloned()
            .ok_or_else(invalid_pack)?;
        let url = file
            .downloads
            .iter()
            .find(|url| modrinth.is_allowed_pack_download(url))
            .cloned()
            .ok_or_else(invalid_pack)?;
        files.push(ContentFile {
            url,
            hash: ContentHash::Sha512(sha512),
            size: file.file_size,
            file_name: path,
        });
    }
    if files.len() > MAX_PACK_FILES {
        return Err(invalid_pack());
    }

    let operation_id = Uuid::now_v7();
    let pack_id = Uuid::now_v7();
    let mut ids = vec![pack_id];
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    ContentRepository::insert(
        &mut tx,
        NewContent {
            id: pack_id,
            server_id,
            slot,
            source: "modrinth",
            project_id: Some(&version.project_id),
            version_id: Some(&version.id),
            name: &project.title,
            version_name: Some(&version.version_number),
            icon_url: project.icon_url.as_deref(),
            file_name: &pack.file_name,
            sha512: Some(&pack_sha512),
            sha1: None,
            field_values: &json!({}),
            size_bytes: i64::try_from(pack.size).ok(),
            operation_id: Some(operation_id),
            replaces: None,
            kind: "modpack",
            modpack_id: None,
            state: "installing",
        },
    )
    .await?;
    for file in &files {
        let id = Uuid::now_v7();
        ids.push(id);
        let name = file
            .file_name
            .rsplit('/')
            .next()
            .unwrap_or(&file.file_name)
            .trim_end_matches(".jar")
            .to_owned();
        ContentRepository::insert(
            &mut tx,
            NewContent {
                id,
                server_id,
                slot,
                source: "modpack",
                project_id: None,
                version_id: None,
                name: &name,
                version_name: None,
                icon_url: None,
                file_name: &file.file_name,
                sha512: Some(file.hash.value()),
                sha1: None,
                field_values: &json!({}),
                size_bytes: i64::try_from(file.size).ok(),
                operation_id: Some(operation_id),
                replaces: None,
                kind: "file",
                modpack_id: Some(pack_id),
                state: "installing",
            },
        )
        .await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::ServerModpackInstalled)
            .target("server", server_id)
            .details(json!({
                "slot": slot,
                "project_id": version.project_id,
                "version_id": version.id,
                "game_version": game_version,
                "loader": loader,
                "files": files.len(),
            })),
    )
    .await?;

    let message = ContentInstall {
        server_id,
        operation_id,
        service: plan.service.clone(),
        dir: plan.dir.clone().unwrap_or_default(),
        files,
        root: Some(root),
        archive: Some(ContentFile {
            url: pack.url.clone(),
            hash: ContentHash::Sha512(pack_sha512.clone()),
            size: pack.size,
            file_name: pack.file_name.clone(),
        }),
        extract: None,
    };

    // Le modpack impose la version du jeu et le loader : redéploiement, puis installation dès que
    // le nœud est libre.
    let settings = &server.settings;
    let mut changes = serde_json::Map::new();
    if settings.get(&version_key).and_then(|v| v.as_str()) != Some(game_version.as_str()) {
        changes.insert(version_key, json!(game_version));
    }
    if settings.get(&loader_key).and_then(|v| v.as_str()) != Some(loader) {
        changes.insert(loader_key, json!(loader));
    }
    if changes.is_empty() {
        if let Err(error) = send_pack(state, &server, message).await {
            fail_pack(state, &ids, &error).await?;
            return Err(error);
        }
    } else {
        super::super::manage::update_settings(state, context, server_id, &changes).await?;
        let (task_state, server, ids) = (state.clone(), server.clone(), ids.clone());
        tokio::spawn(async move {
            let mut outcome = Err(ServersError::Conflict(ErrorCode::ServerBusy));
            for _ in 0..PACK_RETRIES {
                outcome = send_pack(&task_state, &server, message.clone()).await;
                match &outcome {
                    Err(ServersError::Conflict(ErrorCode::ServerBusy)) => {
                        tokio::time::sleep(PACK_RETRY_DELAY).await;
                    }
                    _ => break,
                }
            }
            if let Err(error) = outcome {
                tracing::warn!(%server_id, %error, "modpack install not sent");
                if let Err(db) = fail_pack(&task_state, &ids, &error).await {
                    tracing::error!(%server_id, %db, "failed to record a modpack failure");
                }
            }
        });
    }
    let mut conn = state.pool.acquire().await?;
    Ok(ContentRepository::of_slot(&mut conn, server_id, slot)
        .await?
        .into_iter()
        .filter(|c| c.id == pack_id || c.modpack_id == Some(pack_id))
        .collect())
}

async fn send_pack(
    state: &ServersState,
    server: &ServerRecord,
    message: ContentInstall,
) -> Result<(), ServersError> {
    let result = state
        .commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::ContentInstall(message),
        )
        .await?;
    accepted(result)
}

/// Modpack jamais parti vers le nœud : ses lignes passent en échec, et c'est journalisé.
async fn fail_pack(
    state: &ServersState,
    ids: &[Uuid],
    error: &ServersError,
) -> Result<(), sqlx::Error> {
    let mut tx = AuditedTx::begin(&state.pool, &AuditContext::system()).await?;
    ContentRepository::fail(&mut tx, ids, refusal(error)).await?;
    tx.commit(
        AuditEvent::failure(AuditAction::ServerModpackInstalled)
            .details(json!({ "reason": refusal(error) })),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modpack_names_its_loader_by_its_dependency() {
        for (dependency, loader) in [
            ("fabric-loader", "fabric"),
            ("quilt-loader", "quilt"),
            ("forge", "forge"),
            ("neoforge", "neoforge"),
        ] {
            let dependencies = HashMap::from([
                ("minecraft".to_owned(), "1.21.4".to_owned()),
                (dependency.to_owned(), "1.0.0".to_owned()),
            ]);
            assert_eq!(pack_loader(&dependencies), Some(loader));
        }
        let vanilla = HashMap::from([("minecraft".to_owned(), "1.21.4".to_owned())]);
        assert_eq!(pack_loader(&vanilla), None);
    }
}
