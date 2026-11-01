//! Côté daemon du socket Unix (ADR-0047) : le processus SFTP y demande authentifications et
//! baux, relayés à l'API, et y dépose ses événements d'audit, écrits dans la file locale puis
//! remis à l'API.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kuro_protocol::api::sftp::{SftpDenial, SftpGrant, SftpLeaseResult};
use kuro_protocol::daemon::sftp::{SftpAudit, SftpAuth, SftpLease, SftpLeaseRequest};
use kuro_protocol::{ApiMessage, DaemonMessage};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::SftpConfig;
use super::ipc::{
    AuthOutcome, BridgeMessage, Grant, WorkerMessage, WriteRefusal, read_frame, write_frame,
};
use super::spool::Spool;
use crate::link::ApiRequests;
use crate::server::{CommandError, Manager};

const RETRY: Duration = Duration::from_secs(5);

/// Utilisateur du processus SFTP : `(uid, gid)` lus dans `/etc/passwd`.
fn worker_identity(name: &str) -> Option<(u32, u32)> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != name {
            return None;
        }
        let _password = fields.next()?;
        let uid = fields.next()?.parse().ok()?;
        let gid = fields.next()?.parse().ok()?;
        Some((uid, gid))
    })
}

#[derive(Clone)]
struct Bridge {
    manager: Option<Manager>,
    api: ApiRequests,
    spool: Arc<Spool>,
}

/// Écoute le socket jusqu'à `shutdown`. Seul l'utilisateur du processus SFTP y est admis
/// (`SO_PEERCRED`) quand il est configuré.
pub async fn serve(
    config: &SftpConfig,
    manager: Option<Manager>,
    api: ApiRequests,
    spool: Arc<Spool>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    // Vide (développement, tout en root) : aucune vérification, comme absent.
    let identity = match config
        .worker_user
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        Some(name) => match worker_identity(name) {
            Some(identity) => Some(identity),
            None => {
                return Err(std::io::Error::other(format!(
                    "sftp worker user {name:?} does not exist"
                )));
            }
        },
        None => None,
    };
    let listener = bind(&config.socket, identity)?;
    tracing::info!(socket = %config.socket.display(), "sftp bridge listening");
    let bridge = Bridge {
        manager,
        api,
        spool,
    };
    loop {
        let accepted = tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted,
        };
        let stream = match accepted {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::warn!(%error, "failed to accept sftp worker");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        if let Some((uid, _)) = identity {
            match stream.peer_cred() {
                Ok(peer) if peer.uid() == uid => {}
                Ok(peer) => {
                    tracing::warn!(uid = peer.uid(), "sftp socket: unexpected peer refused");
                    continue;
                }
                Err(error) => {
                    tracing::warn!(%error, "sftp socket: peer credentials unavailable");
                    continue;
                }
            }
        }
        tokio::spawn(bridge.clone().connection(stream, shutdown.clone()));
    }
}

/// Socket neuf : `0660` pour le groupe du processus SFTP, sinon `0600`.
fn bind(path: &Path, identity: Option<(u32, u32)>) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    match identity {
        Some((_, gid)) => {
            std::os::unix::fs::chown(path, None, Some(gid))?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
        }
        None => std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?,
    }
    Ok(listener)
}

impl Bridge {
    async fn connection(self, stream: UnixStream, shutdown: CancellationToken) {
        tracing::info!("sftp worker connected");
        let (mut reader, mut writer) = stream.into_split();
        let (replies, mut outgoing) = mpsc::channel::<BridgeMessage>(64);
        let writer_task = tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if let Err(error) = write_frame(&mut writer, &message).await {
                    tracing::warn!(%error, "failed to write to the sftp worker");
                    return;
                }
            }
        });
        loop {
            let frame = tokio::select! {
                () = shutdown.cancelled() => break,
                frame = read_frame::<_, WorkerMessage>(&mut reader) => frame,
            };
            let message = match frame {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "unreadable frame from the sftp worker");
                    break;
                }
            };
            let bridge = self.clone();
            let replies = replies.clone();
            tokio::spawn(async move {
                let reply = bridge.answer(message).await;
                let _ = replies.send(reply).await;
            });
        }
        drop(replies);
        let _ = writer_task.await;
        tracing::info!("sftp worker disconnected");
    }

    async fn answer(&self, message: WorkerMessage) -> BridgeMessage {
        match message {
            WorkerMessage::Auth { request, auth } => BridgeMessage::Auth {
                request,
                outcome: self.authenticate(auth).await,
            },
            WorkerMessage::Lease { request, sessions } => BridgeMessage::Lease {
                request,
                results: self.renew(sessions).await,
            },
            WorkerMessage::Check { request, server_id } => BridgeMessage::Check {
                request,
                refusal: self.write_check(server_id).await,
            },
            WorkerMessage::Audit { request, events } => {
                // Fichiers écrits : l'espace disque est remesuré, comme après une écriture web.
                let written: std::collections::BTreeSet<Uuid> = events
                    .iter()
                    .filter(|event| event.event.is_write())
                    .map(|event| event.server_id)
                    .collect();
                if let Some(manager) = &self.manager {
                    for server_id in written {
                        manager.sftp_written(server_id);
                    }
                }
                let accepted = match self.spool.append(events) {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::error!(%error, "sftp audit events refused");
                        false
                    }
                };
                BridgeMessage::Audit { request, accepted }
            }
        }
    }

    async fn write_check(&self, server_id: Uuid) -> Option<WriteRefusal> {
        let Some(manager) = &self.manager else {
            return Some(WriteRefusal::Unavailable);
        };
        match manager.sftp_write_check(server_id).await {
            Ok(()) => None,
            Err(CommandError::Busy) => Some(WriteRefusal::Busy),
            Err(CommandError::Suspended) => Some(WriteRefusal::Suspended),
            Err(CommandError::DiskQuotaExceeded) => Some(WriteRefusal::DiskQuotaExceeded),
            Err(_) => Some(WriteRefusal::Unavailable),
        }
    }

    /// L'API décide ; le daemon ajoute ce que le serveur déployé expose, et refuse un serveur
    /// qu'il ne connaît pas ou qui est suspendu.
    async fn authenticate(&self, auth: SftpAuth) -> AuthOutcome {
        let denied = |reason| AuthOutcome::Denied { reason };
        let grant = match self.api.request(DaemonMessage::SftpAuth(auth)).await {
            Ok(ApiMessage::SftpGrant(grant)) => grant,
            Ok(_) => return denied(SftpDenial::Other),
            Err(error) => {
                tracing::warn!(%error, "sftp authentication not relayed");
                return denied(SftpDenial::Other);
            }
        };
        let (server_id, principal, credential, write, lease_secs) = match grant {
            SftpGrant::Granted {
                server_id,
                principal,
                credential,
                write,
                lease_secs,
            } => (server_id, principal, credential, write, lease_secs),
            SftpGrant::Denied { reason } => return denied(reason),
        };
        let layout = match &self.manager {
            Some(manager) => manager.sftp_layout(server_id),
            None => Err(CommandError::UnknownServer),
        };
        match layout {
            Ok(layout) => AuthOutcome::Granted(Box::new(Grant {
                server_id,
                principal,
                credential,
                write,
                lease_secs,
                layout,
                history: self
                    .manager
                    .as_ref()
                    .map(Manager::history_policy)
                    .unwrap_or_default(),
            })),
            Err(CommandError::Suspended) => denied(SftpDenial::ServerSuspended),
            Err(_) => denied(SftpDenial::UnknownServer),
        }
    }

    /// Bail renouvelé si l'API l'accorde et que le serveur est toujours là, actif.
    async fn renew(&self, sessions: Vec<SftpLeaseRequest>) -> Vec<SftpLeaseResult> {
        let servers: HashMap<Uuid, Uuid> = sessions
            .iter()
            .map(|session| (session.session_id, session.server_id))
            .collect();
        let results = match self
            .api
            .request(DaemonMessage::SftpLease(SftpLease { sessions }))
            .await
        {
            Ok(ApiMessage::SftpLeases(leases)) => leases.results,
            Ok(_) => Vec::new(),
            Err(error) => {
                tracing::warn!(%error, "sftp leases not renewed");
                Vec::new()
            }
        };
        results
            .into_iter()
            .filter_map(|mut result| {
                let server_id = servers.get(&result.session_id)?;
                let active = self
                    .manager
                    .as_ref()
                    .is_some_and(|manager| manager.sftp_layout(*server_id).is_ok());
                result.granted &= active;
                Some(result)
            })
            .collect()
    }
}

/// Remet les lots de la file à l'API, du plus ancien au plus récent ; réessaie après une
/// coupure. S'arrête avec `shutdown`.
pub async fn forward_audit(spool: Arc<Spool>, api: ApiRequests, shutdown: CancellationToken) {
    loop {
        loop {
            let batch = match spool.oldest() {
                Ok(Some(batch)) => batch,
                Ok(None) => break,
                Err(error) => {
                    tracing::error!(%error, "sftp audit spool unreadable");
                    break;
                }
            };
            let audit = SftpAudit {
                events: batch.events.clone(),
            };
            match api.request(DaemonMessage::SftpAudit(audit)).await {
                Ok(ApiMessage::SftpAuditAck(_)) => {
                    if let Err(error) = spool.acknowledge(batch) {
                        tracing::error!(%error, "sftp audit batch not removed");
                        break;
                    }
                }
                Ok(_) | Err(_) => break,
            }
        }
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = spool.arrived.notified() => {}
            () = tokio::time::sleep(RETRY) => {}
        }
    }
}
