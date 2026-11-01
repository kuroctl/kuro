//! Envoi d'un fichier de contenu depuis le panel.

use super::*;

/// Envoi d'un fichier dans l'emplacement (`.jar` venu d'ailleurs) : déposé sous le dossier de
/// l'emplacement, enregistré avec la source `upload` (pas de mise à jour possible).
pub async fn upload<E: std::fmt::Display>(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    slot: &str,
    file_name: &str,
    size: u64,
    body: impl futures_util::Stream<Item = Result<axum::body::Bytes, E>> + Unpin,
) -> Result<ContentRecord, ServersError> {
    let server = get(state, server_id).await?.server;
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    let plan = plan_of(state, &server, slot).await?;
    if !accepts_upload(&plan) {
        return Err(ServersError::Unprocessable(ErrorCode::UploadNotAccepted));
    }
    let valid = kuro_protocol::limits::path_segments(file_name).is_some_and(|s| s.len() == 1)
        && !file_name.starts_with('.')
        && !file_name.ends_with(".disabled");
    if !valid {
        return Err(ServersError::Invalid(crate::error_code::FieldError::new(
            "file_name",
            ErrorCode::InvalidPath,
        )));
    }
    let mut conn = state.pool.acquire().await?;
    let current = ContentRepository::of_slot(&mut conn, server_id, slot).await?;
    drop(conn);
    if current
        .iter()
        .any(|c| c.state != "failed" && c.file_name == file_name)
    {
        return Err(ServersError::Conflict(ErrorCode::FileExists));
    }
    let package = crate::prefabs::application::package(
        &state.pool,
        &server.prefab_id,
        &server.prefab_version,
    )
    .await?;
    let dir = plan.dir.clone().unwrap_or_default();
    let volume_path = package
        .manifest
        .services
        .get(&plan.service)
        .and_then(|service| {
            service.volumes.iter().find_map(|(name, volume)| {
                let root = volume.path.trim_end_matches('/');
                let rest = if dir == root {
                    Some("")
                } else {
                    dir.strip_prefix(&format!("{root}/"))
                }?;
                Some(format!("/{}/{name}/{rest}", plan.service))
            })
        })
        .ok_or(ServersError::Unprocessable(ErrorCode::ContentNotSupported))?;
    let path = format!("{}/{file_name}", volume_path.trim_end_matches('/'));
    let author = context.actor.id.map(|id| kuro_protocol::api::FileAuthor {
        kind: kuro_protocol::api::AuthorKind::Admin,
        id: Some(id),
    });
    let uploaded =
        super::super::files::upload_raw(state, server_id, &path, size, author, body).await?;

    let id = Uuid::now_v7();
    let name = file_name.trim_end_matches(".jar");
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    ContentRepository::insert(
        &mut tx,
        NewContent {
            id,
            server_id,
            slot,
            source: "upload",
            project_id: None,
            version_id: None,
            name,
            version_name: None,
            icon_url: None,
            file_name,
            sha512: None,
            sha1: None,
            field_values: &json!({}),
            size_bytes: i64::try_from(uploaded.size).ok(),
            operation_id: None,
            replaces: None,
            kind: "file",
            modpack_id: None,
            state: "installed",
        },
    )
    .await?;
    if is_live(&server) {
        ServerRepository::defer_changes(&mut tx, server_id, &[id], 0).await?;
    }
    tx.commit(
        AuditEvent::success(AuditAction::ServerContentInstalled)
            .target("server", server_id)
            .details(json!({ "slot": slot, "source": "upload", "file_name": file_name, "sha256": uploaded.sha256 })),
    )
    .await?;
    let mut conn = state.pool.acquire().await?;
    ContentRepository::get(&mut conn, server_id, id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::ContentNotFound))
}
