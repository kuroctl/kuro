//! Session établie : `hello` → `hello.ack`, puis heartbeats et pings jusqu'à la coupure.
//!
//! Les commandes de l'API s'exécutent chacune dans leur tâche : la boucle du lien ne fait que
//! lire, classer et transmettre, pour que heartbeats, console et états continuent pendant une
//! opération longue (suppression récursive, lecture de sauvegarde, dossier énorme).

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use kuro_protocol::api::{ErrorCode, HelloAck};
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use kuro_protocol::daemon::{Heartbeat, Hello};
use kuro_protocol::{
    ApiMessage, DaemonMessage, Envelope, MessageId, PROTOCOL_VERSION, decode, encode,
};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{Instant, MissedTickBehavior, timeout};
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};
use tokio_util::sync::CancellationToken;

use super::backoff::Backoff;
use super::{LinkError, WsStream, host};
use crate::server::{CommandError, Manager, Servers};

const HELLO_ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Commandes exécutées en même temps ; les suivantes attendent leur tour dans leur tâche, sans
/// bloquer le lien. L'API n'en a jamais plus de quelques dizaines en vol par nœud.
const MAX_CONCURRENT_COMMANDS: usize = 64;
/// Réponses prêtes, en attente d'envoi par la boucle du lien.
const REPLY_QUEUE: usize = 64;

/// Retourne `Ok(())` uniquement sur arrêt demandé ; toute autre fin est une erreur à retenter.
pub(super) async fn run(
    mut ws: WsStream,
    shutdown: &CancellationToken,
    backoff: &mut Backoff,
    mut servers: Option<&mut Servers>,
) -> Result<(), LinkError> {
    let hello = Envelope::new(DaemonMessage::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: env!("CARGO_PKG_VERSION").to_owned(),
        host: host::host_info(),
    }));
    send(&mut ws, &hello).await?;
    let ack = timeout(HELLO_ACK_TIMEOUT, wait_ack(&mut ws, hello.id))
        .await
        .map_err(|_| LinkError::Timeout("waiting for hello.ack"))??;
    tracing::info!(node_id = %ack.node_id, "api link established");
    backoff.reset();

    let mut ticker = tokio::time::interval(Duration::from_secs(ack.heartbeat_interval_secs.max(1)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await;
    let unresponsive_after = Duration::from_secs(ack.heartbeat_timeout_secs);
    let mut last_seen = Instant::now();

    // Commandes de cette session : exécutées dans leurs tâches, réponses rendues par ce canal.
    // Une commande encore en cours à la fin de la session va à son terme (le drapeau `busy` du
    // gestionnaire est posé à la main : une annulation le laisserait levé) ; sa réponse, sans
    // destinataire, est perdue, et l'API a de toute façon expiré sa requête.
    let (replies_tx, mut replies) = mpsc::channel(REPLY_QUEUE);
    let commands = Commands {
        manager: servers.as_deref().map(|s| s.manager.clone()),
        replies: replies_tx,
        slots: Arc::new(Semaphore::new(MAX_CONCURRENT_COMMANDS)),
    };

    loop {
        tokio::select! {
            () = shutdown.cancelled() => {
                let _ = ws.close(None).await;
                return Ok(());
            }
            _ = ticker.tick() => {
                // Les pongs comptent : une API muette mais vivante répond toujours aux pings.
                if last_seen.elapsed() > unresponsive_after {
                    return Err(LinkError::Unresponsive);
                }
                send(&mut ws, &Envelope::new(DaemonMessage::Heartbeat(Heartbeat {}))).await?;
                ws.send(Message::Ping(Bytes::new())).await.map_err(Box::new)?;
            }
            message = next_outgoing(&mut servers) => {
                send(&mut ws, &Envelope::new(message)).await?;
            }
            // Jamais `None` : `commands.replies` garde un émetteur jusqu'à la fin de la session.
            Some(reply) = replies.recv() => {
                send(&mut ws, &reply).await?;
            }
            frame = ws.next() => {
                last_seen = Instant::now();
                match frame {
                    None | Some(Ok(Message::Close(_))) => return Err(LinkError::Closed),
                    Some(Err(error)) => return Err(Box::new(error).into()),
                    Some(Ok(Message::Text(text))) => match classify(&text) {
                        Inbound::Stop(error) => return Err(error),
                        Inbound::Command(envelope) => commands.spawn(*envelope),
                        Inbound::Ignored => {}
                    },
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn wait_ack(ws: &mut WsStream, hello_id: MessageId) -> Result<HelloAck, LinkError> {
    loop {
        let text = match ws.next().await {
            None | Some(Ok(Message::Close(_))) => return Err(LinkError::Closed),
            Some(Err(error)) => return Err(Box::new(error).into()),
            Some(Ok(Message::Text(text))) => text,
            Some(Ok(_)) => continue,
        };
        let envelope = decode::<ApiMessage>(text.as_str())?;
        return match envelope.message {
            ApiMessage::HelloAck(ack) if envelope.reply_to == Some(hello_id) => Ok(ack),
            ApiMessage::Error(error) => Err(LinkError::from_api_error(error)),
            // Rien d'autre n'est attendu avant l'acquittement du `hello`.
            _ => Err(LinkError::UnexpectedMessage),
        };
    }
}

/// Suite donnée à un message de l'API, décidée sans attendre.
enum Inbound {
    /// Seule la révocation arrête le lien.
    Stop(LinkError),
    /// Commande à exécuter, qui recevra un `command.result` ou une réponse portant des données.
    Command(Box<Envelope<ApiMessage>>),
    Ignored,
}

/// Prochain message à transmettre : les `server.state` et fins de sauvegarde d'abord, puis la
/// console et les stats des serveurs regardés. En attente indéfinie sans gestion des serveurs.
async fn next_outgoing(servers: &mut Option<&mut Servers>) -> DaemonMessage {
    let Some(servers) = servers else {
        return std::future::pending().await;
    };
    let outgoing = &mut servers.outgoing;
    tokio::select! {
        biased;
        Some(report) = outgoing.events.recv() => DaemonMessage::ServerState(report),
        Some(done) = outgoing.backups.recv() => DaemonMessage::BackupDone(done),
        Some(done) = outgoing.content.recv() => DaemonMessage::ContentDone(done),
        Some(message) = outgoing.streams.recv() => message,
        else => std::future::pending().await,
    }
}

/// Message de l'API pendant la session. Les erreurs non fatales sont journalisées (une erreur
/// fatale est suivie d'une fermeture, traitée par la boucle).
fn classify(text: &Utf8Bytes) -> Inbound {
    let envelope = match decode::<ApiMessage>(text.as_str()) {
        Ok(envelope) => envelope,
        // Message d'une version plus récente de l'API : ignoré plutôt que de couper le lien.
        Err(error) => {
            tracing::warn!(%error, "ignoring unreadable api message");
            return Inbound::Ignored;
        }
    };
    match &envelope.message {
        ApiMessage::Error(error) if error.code == ErrorCode::NodeRevoked => {
            Inbound::Stop(LinkError::Revoked)
        }
        ApiMessage::Error(error) => {
            tracing::warn!(code = ?error.code, message = %error.message, "api reported an error");
            Inbound::Ignored
        }
        ApiMessage::HelloAck(_) => Inbound::Ignored,
        _ => Inbound::Command(Box::new(envelope)),
    }
}

/// Exécution des commandes d'une session, hors de la boucle du lien.
struct Commands {
    /// Absent : le daemon ne gère pas de serveurs (tests du lien sans Docker).
    manager: Option<Manager>,
    replies: mpsc::Sender<Envelope<DaemonMessage>>,
    slots: Arc<Semaphore>,
}

impl Commands {
    fn spawn(&self, envelope: Envelope<ApiMessage>) {
        let manager = self.manager.clone();
        let replies = self.replies.clone();
        let slots = Arc::clone(&self.slots);
        tokio::spawn(async move {
            // Le sémaphore n'est jamais fermé : l'échec est impossible, et vaut un abandon.
            let Ok(_slot) = slots.acquire_owned().await else {
                return;
            };
            let request = envelope.id;
            let outcome = match &manager {
                Some(manager) => execute(envelope.message, manager).await,
                None => Err(CommandError::Internal(
                    "server management is disabled on this daemon".to_owned(),
                )),
            };
            let reply = Envelope::reply(request, reply_of(outcome));
            // Session terminée entre-temps : la réponse n'a plus de destinataire.
            let _ = replies.send(reply).await;
        });
    }
}

/// Réponse à une commande : ses données, `command.result` ok, ou le refus.
fn reply_of(outcome: Result<Option<DaemonMessage>, CommandError>) -> DaemonMessage {
    match outcome {
        Ok(Some(message)) => message,
        Ok(None) => DaemonMessage::CommandResult(CommandResult::Ok),
        Err(error) => {
            tracing::warn!(%error, "command refused");
            DaemonMessage::CommandResult(CommandResult::Error {
                code: match error {
                    CommandError::InvalidSpec(_) => CommandErrorCode::InvalidSpec,
                    CommandError::UnknownServer => CommandErrorCode::UnknownServer,
                    CommandError::Busy => CommandErrorCode::Busy,
                    CommandError::NotRunning => CommandErrorCode::NotRunning,
                    CommandError::InvalidPath => CommandErrorCode::InvalidPath,
                    CommandError::NotFound => CommandErrorCode::NotFound,
                    CommandError::AlreadyExists => CommandErrorCode::AlreadyExists,
                    CommandError::Conflict => CommandErrorCode::Conflict,
                    CommandError::NotText => CommandErrorCode::NotText,
                    CommandError::TooLarge => CommandErrorCode::TooLarge,
                    CommandError::DiskQuotaExceeded => CommandErrorCode::DiskQuotaExceeded,
                    CommandError::Suspended => CommandErrorCode::Suspended,
                    CommandError::Internal(_) => CommandErrorCode::Internal,
                },
                message: error.to_string(),
            })
        }
    }
}

/// Exécute une commande de serveur ; `Some` pour une réponse qui porte des données.
async fn execute(message: ApiMessage, m: &Manager) -> Result<Option<DaemonMessage>, CommandError> {
    match message {
        ApiMessage::ServerDeploy(deploy) => m.deploy(deploy).map(|()| None),
        ApiMessage::ServerDelete(delete) => m.delete(delete.server_id).map(|()| None),
        ApiMessage::ServerPower(power) => {
            m.power(power.server_id, power.action).await.map(|()| None)
        }
        ApiMessage::ServerLimits(limits) => m.limits(limits).map(|()| None),
        ApiMessage::ServerSuspend(suspend) => m
            .suspend(suspend.server_id, suspend.suspended)
            .map(|()| None),
        ApiMessage::ServerSync(sync) => {
            m.sync(sync.server_ids);
            Ok(None)
        }
        ApiMessage::ServerInstall(install) => m
            .install(install.server_id, install.mode, install.install)
            .await
            .map(|()| None),
        ApiMessage::ServerWatch(watch) => m.watch(watch.server_id, watch.watch).map(|()| None),
        ApiMessage::ServerCommand(command) => m
            .command(command.server_id, &command.line)
            .await
            .map(|()| None),
        ApiMessage::FileList(list) => m
            .file_list(list.server_id, &list.path)
            .await
            .map(|listing| Some(DaemonMessage::FileListing(listing))),
        ApiMessage::FileRead(read) => m
            .file_read(read.server_id, &read.path)
            .await
            .map(|content| Some(DaemonMessage::FileContent(content))),
        ApiMessage::FileWrite(write) => m
            .file_write(write.server_id, &write.path, write.content, write.if_sha256)
            .await
            .map(|()| None),
        ApiMessage::FileMkdir(mkdir) => m
            .file_mkdir(mkdir.server_id, &mkdir.path)
            .await
            .map(|()| None),
        ApiMessage::FileRename(rename) => m
            .file_rename(rename.server_id, &rename.from, &rename.to)
            .await
            .map(|()| None),
        ApiMessage::FileDelete(delete) => m
            .file_delete(delete.server_id, &delete.path)
            .await
            .map(|()| None),
        ApiMessage::BackupCreate(create) => m.backup_create(create).await.map(|()| None),
        ApiMessage::BackupRestore(restore) => m
            .backup_restore(restore.server_id, restore.backup_id, restore.mode)
            .await
            .map(|()| None),
        ApiMessage::BackupDelete(delete) => m
            .backup_delete(delete.server_id, delete.backup_id)
            .await
            .map(|()| None),
        ApiMessage::BackupList(list) => m
            .backup_list(list.server_id)
            .await
            .map(|listing| Some(DaemonMessage::BackupListing(listing))),
        ApiMessage::BackupRead(read) => m
            .backup_read(read.server_id, read.backup_id, read.offset)
            .await
            .map(|chunk| Some(DaemonMessage::BackupChunk(chunk))),
        ApiMessage::FileUpload(upload) => m.file_upload(upload).await.map(|()| None),
        ApiMessage::FileUploadChunk(chunk) => m.file_upload_chunk(chunk).await.map(|()| None),
        ApiMessage::ContentInstall(install) => m.content_install(install).await.map(|()| None),
        ApiMessage::ContentRemove(remove) => m.content_remove(remove).await.map(|()| None),
        ApiMessage::ContentToggle(toggle) => m.content_toggle(toggle).await.map(|()| None),
        ApiMessage::HelloAck(_) | ApiMessage::Error(_) => Ok(None),
    }
}

async fn send(ws: &mut WsStream, envelope: &Envelope<DaemonMessage>) -> Result<(), LinkError> {
    let text = encode(envelope)?;
    ws.send(Message::text(text)).await.map_err(Box::new)?;
    Ok(())
}
