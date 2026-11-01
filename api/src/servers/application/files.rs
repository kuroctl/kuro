//! Fichiers d'un serveur (ADR-0015) : relais vers le nœud, chemins vérifiés des deux côtés.
//! Les écritures sont auditées sans leur contenu.

use axum::body::Bytes;
use base64::Engine;
use futures_util::{Stream, StreamExt};
use kuro_protocol::api::{
    AuthorKind, FileAuthor, FileDelete, FileHistory, FileList, FileMkdir, FileRead, FileRename,
    FileRestore, FileUpload, FileUploadChunk, FileVersionRead, FileWrite,
};
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use kuro_protocol::daemon::{FileContent, FileListing, FileVersions};
use kuro_protocol::limits::{
    MAX_MESSAGE_BYTES, MAX_TEXT_FILE_BYTES, MAX_UPLOAD_BYTES, UPLOAD_CHUNK_BYTES, path_segments,
};
use kuro_protocol::{ApiMessage, DaemonMessage};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{ServersError, ServersState, get};
use crate::audit::domain::{ActorKind, AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::{NodeId, NodeStatus};
use crate::servers::domain::ServerState;

/// Marge pour l'enveloppe autour du contenu d'un fichier.
const ENVELOPE_MARGIN: usize = 1024;

pub async fn list(state: &ServersState, id: Uuid, path: &str) -> Result<FileListing, ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, false).await?;
    match request(
        state,
        node,
        ApiMessage::FileList(FileList {
            server_id: id,
            path,
        }),
    )
    .await?
    {
        DaemonMessage::FileListing(listing) => Ok(listing),
        other => Err(unexpected(other)),
    }
}

pub async fn read(state: &ServersState, id: Uuid, path: &str) -> Result<FileContent, ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, false).await?;
    match request(
        state,
        node,
        ApiMessage::FileRead(FileRead {
            server_id: id,
            path,
        }),
    )
    .await?
    {
        DaemonMessage::FileContent(content) => Ok(content),
        other => Err(unexpected(other)),
    }
}

/// Écriture d'un fichier texte ; `if_sha256` protège d'un écrasement (`file_conflict`).
pub async fn write(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    path: &str,
    content: String,
    if_sha256: Option<String>,
) -> Result<(), ServersError> {
    let path = checked(path, "path")?;
    let encoded =
        serde_json::to_string(&content).map_err(|e| ServersError::Internal(e.to_string()))?;
    if content.len() > MAX_TEXT_FILE_BYTES || encoded.len() + ENVELOPE_MARGIN > MAX_MESSAGE_BYTES {
        return Err(ServersError::Unprocessable(ErrorCode::FileTooLarge));
    }
    let size = content.len();
    let node = reachable(state, id, true).await?;
    let message = ApiMessage::FileWrite(FileWrite {
        server_id: id,
        path: path.clone(),
        content,
        if_sha256,
        author: author(context),
    });
    done(request(state, node, message).await?, "path")?;
    // Audit ouvert une fois l'opération faite : aucune connexion tenue pendant l'attente du nœud.
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerFileWritten)
            .target("server", id)
            .details(json!({ "path": path, "size": size, "via": "web" })),
    )
    .await?;
    Ok(())
}

/// Fichier envoyé : taille et SHA-256 calculé par l'API.
#[derive(Debug, Clone)]
pub struct Uploaded {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Envoi d'un fichier binaire (ADR-0020), journalisé (`server.file_uploaded`).
pub async fn upload<E: std::fmt::Display>(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    path: &str,
    size: u64,
    body: impl Stream<Item = Result<Bytes, E>> + Unpin,
) -> Result<Uploaded, ServersError> {
    let uploaded = upload_raw(state, id, path, size, author(context), body).await?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerFileUploaded)
            .target("server", id)
            .details(
                json!({ "path": uploaded.path, "size": uploaded.size, "sha256": uploaded.sha256, "via": "web" }),
            ),
    )
    .await?;
    Ok(uploaded)
}

/// Relais d'un envoi vers le nœud, par morceaux, dans l'ordre : chaque morceau attend la réponse
/// du nœud (mémoire bornée, le navigateur ralentit si le nœud ralentit). Taille annoncée tenue,
/// empreinte vérifiée par le nœud au dernier morceau.
pub(super) async fn upload_raw<E: std::fmt::Display>(
    state: &ServersState,
    id: Uuid,
    path: &str,
    size: u64,
    author: Option<FileAuthor>,
    mut body: impl Stream<Item = Result<Bytes, E>> + Unpin,
) -> Result<Uploaded, ServersError> {
    let path = checked(path, "path")?;
    if size > MAX_UPLOAD_BYTES {
        return Err(ServersError::Unprocessable(ErrorCode::FileTooLarge));
    }
    let node = reachable(state, id, true).await?;
    super::manage::ensure_below_quota(&get(state, id).await?.server)?;
    let upload_id = Uuid::now_v7();
    done(
        request(
            state,
            node,
            ApiMessage::FileUpload(FileUpload {
                server_id: id,
                upload_id,
                path: path.clone(),
                size,
                author,
            }),
        )
        .await?,
        "path",
    )?;

    let mut hasher = Sha256::new();
    let mut buffer: Vec<u8> = Vec::with_capacity(UPLOAD_CHUNK_BYTES);
    let mut offset: u64 = 0;
    let mut received: u64 = 0;
    let send = |chunk: Vec<u8>, offset: u64, last: Option<String>| {
        let message = ApiMessage::FileUploadChunk(FileUploadChunk {
            server_id: id,
            upload_id,
            offset,
            data: base64::engine::general_purpose::STANDARD.encode(&chunk),
            eof: last.is_some(),
            sha256: last,
        });
        async move { done(request(state, node, message).await?, "path") }
    };
    while let Some(piece) = body.next().await {
        let piece = piece.map_err(|e| {
            ServersError::Invalid(
                FieldError::new("body", ErrorCode::InvalidBody).param("reason", e.to_string()),
            )
        })?;
        received += piece.len() as u64;
        if received > size {
            return Err(ServersError::Unprocessable(ErrorCode::FileTooLarge));
        }
        hasher.update(&piece);
        let mut rest: &[u8] = &piece;
        while !rest.is_empty() {
            let take = rest.len().min(UPLOAD_CHUNK_BYTES - buffer.len());
            buffer.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            // Morceau plein, et la suite existe (dans ce bloc ou à venir) : il part ; le dernier
            // attend la fin pour porter l'empreinte.
            if buffer.len() == UPLOAD_CHUNK_BYTES && (!rest.is_empty() || received < size) {
                let chunk = std::mem::replace(&mut buffer, Vec::with_capacity(UPLOAD_CHUNK_BYTES));
                let len = chunk.len() as u64;
                send(chunk, offset, None).await?;
                offset += len;
            }
        }
    }
    if received != size {
        return Err(ServersError::Invalid(FieldError::new(
            "body",
            ErrorCode::InvalidBody,
        )));
    }
    let sha256 = kuro_runtime::tls::hex(&hasher.finalize());
    send(buffer, offset, Some(sha256.clone())).await?;
    Ok(Uploaded { path, size, sha256 })
}

pub async fn mkdir(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    path: &str,
) -> Result<(), ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, true).await?;
    let message = ApiMessage::FileMkdir(FileMkdir {
        server_id: id,
        path: path.clone(),
    });
    done(request(state, node, message).await?, "path")?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerDirectoryCreated)
            .target("server", id)
            .details(json!({ "path": path, "via": "web" })),
    )
    .await?;
    Ok(())
}

pub async fn rename(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    from: &str,
    to: &str,
) -> Result<(), ServersError> {
    let from = checked(from, "from")?;
    let to = checked(to, "to")?;
    let node = reachable(state, id, true).await?;
    let message = ApiMessage::FileRename(FileRename {
        server_id: id,
        from: from.clone(),
        to: to.clone(),
    });
    done(request(state, node, message).await?, "to")?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerFileRenamed)
            .target("server", id)
            .details(json!({ "from": from, "to": to, "via": "web" })),
    )
    .await?;
    Ok(())
}

/// Supprime un fichier, un lien, ou un dossier et tout son contenu.
pub async fn delete(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    path: &str,
) -> Result<(), ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, true).await?;
    let message = ApiMessage::FileDelete(FileDelete {
        server_id: id,
        path: path.clone(),
        author: author(context),
    });
    done(request(state, node, message).await?, "path")?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerFileDeleted)
            .target("server", id)
            .details(json!({ "path": path, "via": "web" })),
    )
    .await?;
    Ok(())
}

/// Versions gardées d'un fichier (ADR-0049), de la plus récente à la plus ancienne.
pub async fn history(
    state: &ServersState,
    id: Uuid,
    path: &str,
) -> Result<FileVersions, ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, false).await?;
    match request(
        state,
        node,
        ApiMessage::FileHistory(FileHistory {
            server_id: id,
            path,
        }),
    )
    .await?
    {
        DaemonMessage::FileVersions(versions) => Ok(versions),
        other => Err(unexpected(other)),
    }
}

/// Contenu texte d'une version.
pub async fn version(
    state: &ServersState,
    id: Uuid,
    path: &str,
    version: Uuid,
) -> Result<FileContent, ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, false).await?;
    match request(
        state,
        node,
        ApiMessage::FileVersionRead(FileVersionRead {
            server_id: id,
            path,
            version,
        }),
    )
    .await?
    {
        DaemonMessage::FileContent(content) => Ok(content),
        other => Err(unexpected(other)),
    }
}

/// Remet une version à la place du fichier, journalisé (`server.file_restored`). La version
/// remplacée est gardée par le nœud : la restauration s'annule.
pub async fn restore(
    state: &ServersState,
    context: &AuditContext,
    id: Uuid,
    path: &str,
    version: Uuid,
) -> Result<(), ServersError> {
    let path = checked(path, "path")?;
    let node = reachable(state, id, true).await?;
    let message = ApiMessage::FileRestore(FileRestore {
        server_id: id,
        path: path.clone(),
        version,
        author: author(context),
    });
    done(request(state, node, message).await?, "path")?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerFileRestored)
            .target("server", id)
            .details(json!({ "path": path, "version": version, "via": "web" })),
    )
    .await?;
    Ok(())
}

/// Fichiers supprimés d'un dossier dont une version est encore gardée (ADR-0049).
pub async fn deleted(
    state: &ServersState,
    id: Uuid,
    dir: &str,
) -> Result<kuro_protocol::daemon::DeletedFiles, ServersError> {
    let dir = checked(dir, "path")?;
    let node = reachable(state, id, false).await?;
    match request(
        state,
        node,
        ApiMessage::FileHistoryDeleted(kuro_protocol::api::FileHistoryDeleted {
            server_id: id,
            dir,
        }),
    )
    .await?
    {
        DaemonMessage::DeletedFiles(files) => Ok(files),
        other => Err(unexpected(other)),
    }
}

/// Rétention de l'historique telle que réglée dans le projet.
pub fn history_policy(days: i32, max_mb: i32) -> kuro_protocol::api::HistoryPolicy {
    kuro_protocol::api::HistoryPolicy {
        days: u32::try_from(days).unwrap_or(0),
        max_mb: u32::try_from(max_mb).unwrap_or(1),
    }
}

/// Envoie la nouvelle rétention à chaque nœud en ligne, où qu'il soit tenu (ADR-0037), en tâche
/// de fond. Un nœud hors ligne la reçoit dans `hello.ack` à sa reconnexion.
pub fn send_history_policy(state: &ServersState, days: i32, max_mb: i32) {
    let state = state.clone();
    let policy = history_policy(days, max_mb);
    tokio::spawn(async move {
        let nodes = match sqlx::query_scalar!("SELECT id FROM nodes WHERE status = 'online'")
            .fetch_all(&state.pool)
            .await
        {
            Ok(nodes) => nodes,
            Err(error) => {
                tracing::error!(%error, "file history policy not sent");
                return;
            }
        };
        for node in nodes {
            let message = ApiMessage::NodeSettings(kuro_protocol::api::NodeSettings {
                file_history: policy,
            });
            if let Err(error) = state.commands.request(NodeId::from(node), message).await {
                tracing::warn!(%node, ?error, "file history policy not delivered");
            }
        }
    });
}

/// Auteur d'une écriture, gardé avec la version qu'elle remplace : l'identifiant de l'acteur,
/// jamais son e-mail. Un agent IA écrit au nom de son admin.
fn author(context: &AuditContext) -> Option<FileAuthor> {
    let kind = match context.actor.kind {
        ActorKind::Admin => AuthorKind::Admin,
        ActorKind::Customer => AuthorKind::Customer,
        ActorKind::Agent => AuthorKind::Agent,
        ActorKind::System | ActorKind::Cli => AuthorKind::System,
        ActorKind::Node | ActorKind::Anonymous => return None,
    };
    Some(FileAuthor {
        kind,
        id: context.actor.id,
    })
}

/// Chemin normalisé (`/a/b/c`), ou refus sous le champ donné.
fn checked(path: &str, field: &'static str) -> Result<String, ServersError> {
    let segments = path_segments(path)
        .ok_or_else(|| ServersError::Invalid(FieldError::new(field, ErrorCode::InvalidPath)))?;
    Ok(format!("/{}", segments.join("/")))
}

/// Nœud du serveur, en ligne. Une modification attend la fin d'un déploiement ou d'une
/// suppression.
async fn reachable(state: &ServersState, id: Uuid, write: bool) -> Result<NodeId, ServersError> {
    let server = get(state, id).await?.server;
    let busy = server.state == ServerState::Deleting.as_str()
        || (write
            && (server.state == ServerState::Deploying.as_str()
                || server.state == ServerState::Installing.as_str()
                || server.state == ServerState::Restoring.as_str()));
    if busy {
        return Err(ServersError::Conflict(ErrorCode::ServerBusy));
    }
    super::manage::ensure_not_suspended(&server)?;
    if server.node_status.parse::<NodeStatus>().ok() != Some(NodeStatus::Online) {
        return Err(ServersError::Conflict(ErrorCode::NodeOffline));
    }
    Ok(NodeId::from(server.node_id))
}

async fn request(
    state: &ServersState,
    node: NodeId,
    message: ApiMessage,
) -> Result<DaemonMessage, ServersError> {
    match state.commands.request(node, message).await? {
        DaemonMessage::CommandResult(CommandResult::Error { code, message }) => {
            Err(refused(code, message, "path"))
        }
        reply => Ok(reply),
    }
}

/// Opération sans données en retour : `command.result` attendu.
fn done(reply: DaemonMessage, field: &'static str) -> Result<(), ServersError> {
    match reply {
        DaemonMessage::CommandResult(CommandResult::Ok) => Ok(()),
        DaemonMessage::CommandResult(CommandResult::Error { code, message }) => {
            Err(refused(code, message, field))
        }
        other => Err(unexpected(other)),
    }
}

/// Refus du nœud, traduit en erreur stable (ADR-0012).
fn refused(code: CommandErrorCode, message: String, field: &'static str) -> ServersError {
    match code {
        CommandErrorCode::InvalidPath => {
            ServersError::Invalid(FieldError::new(field, ErrorCode::InvalidPath))
        }
        CommandErrorCode::NotFound | CommandErrorCode::UnknownServer => {
            ServersError::Missing(ErrorCode::FileNotFound)
        }
        CommandErrorCode::AlreadyExists => ServersError::Conflict(ErrorCode::FileExists),
        CommandErrorCode::Conflict => ServersError::Conflict(ErrorCode::FileConflict),
        CommandErrorCode::NotText => ServersError::Unprocessable(ErrorCode::FileNotText),
        CommandErrorCode::TooLarge => ServersError::Unprocessable(ErrorCode::FileTooLarge),
        CommandErrorCode::Busy => ServersError::Conflict(ErrorCode::ServerBusy),
        CommandErrorCode::DiskQuotaExceeded => ServersError::Conflict(ErrorCode::DiskQuotaExceeded),
        CommandErrorCode::Suspended => ServersError::Conflict(ErrorCode::ServerSuspended),
        code => ServersError::Daemon {
            code: ErrorCode::DaemonError,
            message: format!("{code:?}: {message}"),
        },
    }
}

fn unexpected(reply: DaemonMessage) -> ServersError {
    ServersError::Daemon {
        code: ErrorCode::DaemonError,
        message: format!("unexpected reply: {reply:?}"),
    }
}
