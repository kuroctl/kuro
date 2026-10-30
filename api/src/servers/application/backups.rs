//! Sauvegardes des serveurs (ADR-0018). Les archives restent sur le nœud ; la base garde leur
//! état. La rotation a lieu quand une nouvelle sauvegarde réussit : on ne perd jamais une
//! ancienne sauvegarde pour une nouvelle qui échoue.

use std::collections::BTreeMap;

use axum::body::Bytes;
use base64::Engine;
use futures_util::{Stream, StreamExt, stream};
use kuro_protocol::api::{
    BackupCreate, BackupDelete, BackupList, BackupRead, BackupRestore, RestoreMode,
};
use kuro_protocol::daemon::{BackupDone, BackupOutcome, BackupState};
use kuro_protocol::limits::BACKUP_CHUNK_BYTES;
use kuro_protocol::{ApiMessage, DaemonMessage};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::deploy::accepted;
use super::manage::{ensure_below_quota, ensure_not_suspended, ensure_ready};
use super::{ServersError, ServersState};
use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::error_code::{ErrorCode, FieldError};
use crate::nodes::domain::NodeId;
use crate::prefabs::application as prefabs;
use crate::servers::domain::NodeCommands;
use crate::servers::infra::{BackupRecord, BackupRepository, ServerRecord, ServerRepository};

/// Morceaux demandés d'avance au nœud pendant un téléchargement.
const READ_AHEAD: usize = 4;
const MAX_NAME_CHARS: usize = 64;

/// Serveur et ses sauvegardes, de la plus récente à la plus ancienne.
pub async fn list(
    state: &ServersState,
    server_id: Uuid,
) -> Result<(ServerRecord, Vec<BackupRecord>), ServersError> {
    let mut conn = state.pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, server_id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let backups = BackupRepository::of_server(&mut conn, server_id).await?;
    Ok((server, backups))
}

/// Lance une sauvegarde. Refusée si toutes les places sont prises par des sauvegardes
/// verrouillées ; sinon la plus ancienne non verrouillée sera remplacée quand celle-ci réussira.
pub async fn create(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    name: Option<&str>,
) -> Result<BackupRecord, ServersError> {
    let name = match name.map(str::trim) {
        None | Some("") => default_name(),
        Some(name) if crate::validation::is_display_text(name, 1, MAX_NAME_CHARS) => {
            name.to_owned()
        }
        Some(_) => {
            return Err(ServersError::Invalid(FieldError::new(
                "name",
                ErrorCode::InvalidBackupName,
            )));
        }
    };
    // Le prefab est lu avant la transaction : une seule connexion à la fois par requête.
    let known = super::get(state, server_id).await?.server;
    let package = prefabs::package(&state.pool, &known.prefab_id, &known.prefab_version).await?;
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    if !ServerRepository::lock(&mut tx, server_id).await? {
        return Err(ServersError::NotFound);
    }
    let server = ServerRepository::get(&mut tx, server_id)
        .await?
        .ok_or(ServersError::NotFound)?;
    if server.backup_limit == 0 {
        return Err(ServersError::Conflict(ErrorCode::BackupsDisabled));
    }
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    ensure_below_quota(&server)?;
    let backups = BackupRepository::of_server(&mut tx, server_id).await?;
    if backups.iter().any(|b| b.state == "creating") {
        return Err(ServersError::Conflict(ErrorCode::BackupInProgress));
    }
    let locked = backups
        .iter()
        .filter(|b| b.state == "ready" && b.locked)
        .count();
    if locked >= usize::try_from(server.backup_limit).unwrap_or(0) {
        return Err(ServersError::Conflict(ErrorCode::BackupLimitReached));
    }
    let plan = kuro_prefab::resolve_backup(&package);

    let backup_id = Uuid::now_v7();
    BackupRepository::insert(&mut tx, backup_id, server_id, &name).await?;
    // Validée avant l'envoi : le nœud peut terminer avant notre réponse.
    tx.commit(
        AuditEvent::success(AuditAction::ServerBackupCreated)
            .target("server", server_id)
            .details(json!({ "backup_id": backup_id, "name": name })),
    )
    .await?;

    let sent = state
        .commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::BackupCreate(BackupCreate {
                server_id,
                backup_id,
                ignore: plan.ignore,
                before: plan.before,
                after: plan.after,
            }),
        )
        .await
        .map_err(ServersError::from)
        .and_then(accepted);
    if let Err(error) = sent {
        // Refus du nœud (quota, opération en cours…) : tracé, la sauvegarde passe en échec.
        let mut tx = AuditedTx::begin(&state.pool, context).await?;
        BackupRepository::complete(&mut tx, backup_id, server.node_id, Err(&error.to_string()))
            .await?;
        tx.commit(
            AuditEvent::failure(AuditAction::ServerBackupCreated)
                .target("server", server_id)
                .details(json!({ "backup_id": backup_id, "reason": refusal(&error) })),
        )
        .await?;
        return Err(error);
    }
    get(state, server_id, backup_id).await
}

pub async fn get(
    state: &ServersState,
    server_id: Uuid,
    backup_id: Uuid,
) -> Result<BackupRecord, ServersError> {
    let mut conn = state.pool.acquire().await?;
    BackupRepository::get(&mut conn, server_id, backup_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::BackupNotFound))
}

/// Verrouille (jamais supprimée, ni par la rotation) ou déverrouille.
pub async fn set_locked(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    backup_id: Uuid,
    locked: bool,
) -> Result<BackupRecord, ServersError> {
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    BackupRepository::get(&mut tx, server_id, backup_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::BackupNotFound))?;
    BackupRepository::set_locked(&mut tx, backup_id, locked).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerBackupLocked)
            .target("server", server_id)
            .details(json!({ "backup_id": backup_id, "locked": locked })),
    )
    .await?;
    get(state, server_id, backup_id).await
}

/// Restaure : le nœud arrête le serveur, puis l'état suit (`restoring`, puis `offline`).
pub async fn restore(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    backup_id: Uuid,
    mode: RestoreMode,
) -> Result<(), ServersError> {
    let mut conn = state.pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, server_id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let backup = BackupRepository::get(&mut conn, server_id, backup_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::BackupNotFound))?;
    drop(conn);
    if backup.state != "ready" {
        return Err(ServersError::Conflict(ErrorCode::BackupNotReady));
    }
    ensure_ready(&server)?;
    ensure_not_suspended(&server)?;
    let result = state
        .commands
        .send(
            NodeId::from(server.node_id),
            ApiMessage::BackupRestore(BackupRestore {
                server_id,
                backup_id,
                mode,
            }),
        )
        .await?;
    accepted(result)?;
    // Audit ouvert une fois accepté : aucune connexion tenue pendant l'attente du nœud.
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerBackupRestored)
            .target("server", server_id)
            .details(json!({ "backup_id": backup_id, "mode": mode_name(mode) })),
    )
    .await?;
    Ok(())
}

/// Supprime une sauvegarde non verrouillée. Une sauvegarde en échec n'a pas d'archive : seule
/// sa ligne disparaît.
pub async fn delete(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    backup_id: Uuid,
) -> Result<(), ServersError> {
    let mut conn = state.pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, server_id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let backup = BackupRepository::get(&mut conn, server_id, backup_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::BackupNotFound))?;
    drop(conn);
    if backup.locked {
        return Err(ServersError::Conflict(ErrorCode::BackupLocked));
    }
    if backup.state == "creating" {
        return Err(ServersError::Conflict(ErrorCode::BackupInProgress));
    }
    if backup.state == "ready" {
        let result = state
            .commands
            .send(
                NodeId::from(server.node_id),
                ApiMessage::BackupDelete(BackupDelete {
                    server_id,
                    backup_id,
                }),
            )
            .await?;
        accepted(result)?;
    }
    // L'archive est partie du nœud : la ligne suit, sans connexion tenue pendant l'attente.
    let mut tx = AuditedTx::begin(&state.pool, context).await?;
    BackupRepository::delete(&mut tx, backup_id).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerBackupDeleted)
            .target("server", server_id)
            .details(json!({ "backup_id": backup_id, "cause": "admin" })),
    )
    .await?;
    Ok(())
}

/// Ticket à usage unique du téléchargement, tracé dans l'audit.
pub async fn download_ticket(
    state: &ServersState,
    context: &AuditContext,
    server_id: Uuid,
    backup_id: Uuid,
    admin_id: Uuid,
) -> Result<String, ServersError> {
    let backup = get(state, server_id, backup_id).await?;
    ensure_not_suspended(&super::get(state, server_id).await?.server)?;
    if backup.state != "ready" {
        return Err(ServersError::Conflict(ErrorCode::BackupNotReady));
    }
    let ticket =
        super::tickets::issue_download(&state.pool, server_id, backup_id, admin_id).await?;
    let tx = AuditedTx::begin(&state.pool, context).await?;
    tx.commit(
        AuditEvent::success(AuditAction::ServerBackupDownloaded)
            .target("server", server_id)
            .details(json!({ "backup_id": backup_id })),
    )
    .await?;
    Ok(ticket)
}

/// Archive à envoyer au navigateur.
pub struct Download {
    pub file_name: String,
    pub size: u64,
    pub body: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>,
}

/// Archive tirée du nœud morceau par morceau ; le sha256 est vérifié à la fin (connexion coupée
/// s'il diffère).
pub async fn download(
    state: &ServersState,
    server_id: Uuid,
    backup_id: Uuid,
) -> Result<Download, ServersError> {
    let mut conn = state.pool.acquire().await?;
    let server = ServerRepository::get(&mut conn, server_id)
        .await?
        .ok_or(ServersError::NotFound)?;
    let backup = BackupRepository::get(&mut conn, server_id, backup_id)
        .await?
        .ok_or(ServersError::Missing(ErrorCode::BackupNotFound))?;
    drop(conn);
    let (Some(size), Some(expected)) = (backup.size_bytes, backup.sha256.clone()) else {
        return Err(ServersError::Conflict(ErrorCode::BackupNotReady));
    };
    let size = u64::try_from(size).unwrap_or(0);
    let node = NodeId::from(server.node_id);
    let commands = state.commands.clone();
    let chunk = BACKUP_CHUNK_BYTES as u64;
    let offsets = (0..size.div_ceil(chunk)).map(move |index| index * chunk);
    let reads = stream::iter(offsets)
        .map(move |offset| {
            let commands = commands.clone();
            async move {
                let reply = commands
                    .request(
                        node,
                        ApiMessage::BackupRead(BackupRead {
                            server_id,
                            backup_id,
                            offset,
                        }),
                    )
                    .await
                    .map_err(|f| std::io::Error::other(f.to_string()))?;
                let DaemonMessage::BackupChunk(chunk) = reply else {
                    return Err(std::io::Error::other("unexpected reply to backup.read"));
                };
                let data = base64::engine::general_purpose::STANDARD
                    .decode(chunk.data)
                    .map_err(std::io::Error::other)?;
                let expected_len = usize::try_from(size.saturating_sub(offset).min(chunk_len()))
                    .unwrap_or(usize::MAX);
                if data.len() != expected_len {
                    return Err(std::io::Error::other("short backup chunk"));
                }
                Ok(Bytes::from(data))
            }
        })
        .buffered(READ_AHEAD);
    let mut hasher = Some(Sha256::new());
    let mut sent = 0_u64;
    let body = reads.map(move |chunk| {
        let chunk = chunk?;
        sent += chunk.len() as u64;
        if let Some(h) = hasher.as_mut() {
            h.update(&chunk);
        }
        if sent == size
            && let Some(h) = hasher.take()
        {
            let actual: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if actual != expected {
                tracing::error!(%server_id, %backup_id, "backup checksum mismatch during download");
                return Err(std::io::Error::other("backup checksum mismatch"));
            }
        }
        Ok(chunk)
    });
    Ok(Download {
        file_name: file_name(&server.name, &backup),
        size,
        body: Box::pin(body),
    })
}

fn chunk_len() -> u64 {
    BACKUP_CHUNK_BYTES as u64
}

/// Fin d'archivage rapportée par un nœud, pour l'un de ses serveurs seulement.
pub async fn record_done(
    pool: &PgPool,
    commands: &dyn NodeCommands,
    node: NodeId,
    done: BackupDone,
) -> Result<(), ServersError> {
    let failure: Option<String> = match &done.outcome {
        BackupOutcome::Failed { reason } => Some(
            reason
                .chars()
                .filter(|c| !c.is_control())
                .take(500)
                .collect(),
        ),
        BackupOutcome::Ok { .. } => None,
    };
    let outcome = match (&done.outcome, &failure) {
        (BackupOutcome::Ok { size, sha256 }, _) => {
            Ok((i64::try_from(*size).unwrap_or(i64::MAX), sha256.as_str()))
        }
        (BackupOutcome::Failed { .. }, failure) => Err(failure.as_deref().unwrap_or_default()),
    };
    let mut conn = pool.acquire().await?;
    let updated =
        BackupRepository::complete(&mut conn, done.backup_id, node.as_uuid(), outcome).await?;
    drop(conn);
    if !updated {
        tracing::warn!(%node, backup_id = %done.backup_id, "ignored backup report");
        return Ok(());
    }
    if outcome.is_ok() {
        rotate(pool, commands, done.server_id).await?;
    }
    Ok(())
}

/// Garde au plus `backup_limit` sauvegardes prêtes : les plus anciennes non verrouillées partent.
async fn rotate(
    pool: &PgPool,
    commands: &dyn NodeCommands,
    server_id: Uuid,
) -> Result<(), ServersError> {
    let mut tx = AuditedTx::begin(pool, &AuditContext::system()).await?;
    if !ServerRepository::lock(&mut tx, server_id).await? {
        return Ok(());
    }
    let Some(server) = ServerRepository::get(&mut tx, server_id).await? else {
        return Ok(());
    };
    let backups = BackupRepository::of_server(&mut tx, server_id).await?;
    let ready: Vec<&BackupRecord> = backups.iter().filter(|b| b.state == "ready").collect();
    let excess = ready
        .len()
        .saturating_sub(usize::try_from(server.backup_limit).unwrap_or(0));
    // De la plus ancienne à la plus récente.
    let victims: Vec<Uuid> = ready
        .iter()
        .rev()
        .filter(|b| !b.locked)
        .take(excess)
        .map(|b| b.id)
        .collect();
    let Some((last, others)) = victims.split_last() else {
        return Ok(());
    };
    let event = |backup_id: Uuid| {
        AuditEvent::success(AuditAction::ServerBackupDeleted)
            .target("server", server_id)
            .details(json!({ "backup_id": backup_id, "cause": "rotation" }))
    };
    for &backup_id in others {
        BackupRepository::delete(&mut tx, backup_id).await?;
        tx.record(event(backup_id)).await?;
    }
    BackupRepository::delete(&mut tx, *last).await?;
    tx.commit(event(*last)).await?;
    for backup_id in victims {
        let deleted = commands
            .send(
                NodeId::from(server.node_id),
                ApiMessage::BackupDelete(BackupDelete {
                    server_id,
                    backup_id,
                }),
            )
            .await;
        if !matches!(deleted, Ok(kuro_protocol::daemon::CommandResult::Ok)) {
            tracing::warn!(%server_id, %backup_id, ?deleted, "rotated backup not deleted from its node");
        }
    }
    Ok(())
}

/// À la reconnexion d'un nœud : chaque sauvegarde restée `creating` (serveur, sauvegarde) est
/// terminée selon ce que le nœud en sait (prête, encore en cours, ou perdue).
pub async fn reconcile(
    pool: &PgPool,
    commands: &dyn NodeCommands,
    node: NodeId,
    creating: Vec<(Uuid, Uuid)>,
) -> Result<(), ServersError> {
    let mut by_server: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    for (server_id, backup_id) in creating {
        by_server.entry(server_id).or_default().push(backup_id);
    }
    for (server_id, backup_ids) in by_server {
        let reply = commands
            .request(node, ApiMessage::BackupList(BackupList { server_id }))
            .await?;
        let DaemonMessage::BackupListing(listing) = reply else {
            continue;
        };
        for backup_id in backup_ids {
            let known = listing.backups.iter().find(|b| b.backup_id == backup_id);
            let outcome = match known.map(|b| &b.state) {
                Some(BackupState::Creating) => continue,
                Some(BackupState::Ready { size, sha256 }) => BackupOutcome::Ok {
                    size: *size,
                    sha256: sha256.clone(),
                },
                None => BackupOutcome::Failed {
                    reason: "interrupted: the node lost this backup".to_owned(),
                },
            };
            record_done(
                pool,
                commands,
                node,
                BackupDone {
                    server_id,
                    backup_id,
                    outcome,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// Code stable d'un refus, pour l'audit (jamais de détail technique).
fn refusal(error: &ServersError) -> &'static str {
    match error {
        ServersError::Conflict(code) | ServersError::Unprocessable(code) => code.as_str(),
        ServersError::Daemon { code, .. } => code.as_str(),
        _ => ErrorCode::InternalError.as_str(),
    }
}

fn default_name() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute()
    )
}

/// `<serveur>-<date>.tar.zst`, réduit aux caractères sûrs dans un en-tête HTTP.
fn file_name(server: &str, backup: &BackupRecord) -> String {
    let safe: String = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let at = backup.created_at;
    format!(
        "{}-{:04}{:02}{:02}-{:02}{:02}.tar.zst",
        safe.trim_matches('-'),
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute()
    )
}

fn mode_name(mode: RestoreMode) -> &'static str {
    match mode {
        RestoreMode::Overwrite => "overwrite",
        RestoreMode::Clean => "clean",
    }
}
