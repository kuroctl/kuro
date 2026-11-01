//! Session d'un nœud : `hello` obligatoire, puis heartbeats jusqu'à la fermeture.

use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures_util::StreamExt;
use kuro_protocol::api::{ErrorCode, HelloAck, ProtocolError};
use kuro_protocol::{
    ApiMessage, CodecError, DaemonMessage, Envelope, MessageId, decode, encode, version,
};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use std::collections::HashMap;
use std::sync::Arc;

use kuro_protocol::api::{ServerSync, ServerWatch};
use kuro_protocol::daemon::server::ServerStateReport;
use tokio::sync::mpsc;

/// États en attente d'écriture en base par session ; au-delà, la session attend (contre-pression
/// sur ce nœud seulement).
const STATE_QUEUE: usize = 1024;
/// Messages acceptés d'un nœud par fenêtre de dix secondes : au-delà, il inonde, la session se
/// ferme. Deux cents serveurs très bavards en restent loin (la console est envoyée par lots).
const FLOOD_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
const MAX_MESSAGES_PER_WINDOW: u32 = 50_000;
/// Réponses aux requêtes du nœud en attente d'envoi.
const REPLY_QUEUE: usize = 64;

use super::listener::LinkContext;
use super::registry::{Eviction, Outgoing, Registration};
use crate::nodes::application::LinkStanding;
use crate::nodes::domain::{DisconnectReason, Node, NodeHost};

pub(super) async fn run(
    mut ws: WebSocket,
    node: Node,
    peer: std::net::IpAddr,
    context: &LinkContext,
    shutdown: &CancellationToken,
) {
    let span = tracing::info_span!("daemon_session", node_id = %node.id, node = %node.name);
    async {
        let Some((
            Registration {
                connection,
                evicted,
                commands,
            },
            (assigned, running_backups),
        )) = handshake(&mut ws, &node, peer, context).await
        else {
            close(&mut ws).await;
            return;
        };
        tracing::info!("node online");

        // Serveurs du nœud, lus pendant la poignée de main : il renvoie leur état réel et signale
        // les conteneurs inconnus.
        let sync = Envelope::new(ApiMessage::ServerSync(ServerSync {
            server_ids: assigned,
        }));
        if let Err(error) = send(&mut ws, &sync).await {
            tracing::debug!(%error, "failed to send server.sync");
        }
        // Consoles ouvertes pendant la coupure : le daemon a oublié ses abonnements.
        for server_id in context.servers.watched(node.id) {
            let watch = Envelope::new(ApiMessage::ServerWatch(ServerWatch {
                server_id,
                watch: true,
            }));
            if let Err(error) = send(&mut ws, &watch).await {
                tracing::debug!(%error, "failed to send server.watch");
            }
        }
        // Sauvegardes terminées ou perdues pendant la coupure.
        context.servers.reconcile_backups(node.id, running_backups);
        let reason = serve(&mut ws, &node, context, evicted, commands, shutdown).await;
        close(&mut ws).await;

        if context.registry.unregister(node.id, connection) {
            if let Err(error) = context.presence.offline(node.id).await {
                tracing::error!(%error, "failed to mark node offline");
            }
            tracing::info!(reason, "session closed");
        } else {
            tracing::info!(reason, "connection replaced, node stays online");
        }
    }
    .instrument(span)
    .await;
}

/// Attend le `hello`, le valide, passe le nœud `online` et répond `hello.ack`.
async fn handshake(
    ws: &mut WebSocket,
    node: &Node,
    peer: std::net::IpAddr,
    context: &LinkContext,
) -> Option<(
    Registration,
    (Vec<uuid::Uuid>, Vec<(uuid::Uuid, uuid::Uuid)>),
)> {
    let text = match timeout(context.hello_timeout, next_text(ws)).await {
        Err(_) => {
            reject(
                ws,
                None,
                ErrorCode::UnexpectedMessage,
                "hello not received in time",
            )
            .await;
            return None;
        }
        Ok(Incoming::Text(text)) => text,
        Ok(Incoming::Binary) => {
            reject(
                ws,
                None,
                ErrorCode::InvalidMessage,
                "binary frames are not supported",
            )
            .await;
            return None;
        }
        Ok(Incoming::Closed) => return None,
    };

    let envelope = match decode::<DaemonMessage>(text.as_str()) {
        Ok(envelope) => envelope,
        Err(error) => {
            reject(ws, None, ErrorCode::InvalidMessage, &error.to_string()).await;
            return None;
        }
    };
    let DaemonMessage::Hello(hello) = envelope.message else {
        reject(
            ws,
            Some(envelope.id),
            ErrorCode::UnexpectedMessage,
            "expected hello",
        )
        .await;
        return None;
    };
    if !version::is_supported(hello.protocol_version) {
        let message = format!(
            "protocol version {} is not supported",
            hello.protocol_version
        );
        reject(
            ws,
            Some(envelope.id),
            ErrorCode::UnsupportedProtocolVersion,
            &message,
        )
        .await;
        return None;
    }

    let host = NodeHost {
        daemon_version: hello.daemon_version,
        os: hello.host.os,
        arch: hello.host.arch,
        cpu_cores: hello.host.cpu_cores,
        memory_bytes: hello.host.memory_bytes,
        sftp: hello
            .sftp
            .filter(|endpoint| valid_host_key(&endpoint.host_key_sha256))
            .map(|endpoint| (endpoint.port, endpoint.host_key_sha256)),
        link_address: Some(peer.to_canonical().to_string()),
    };
    // Enregistrer avant de passer `online` : l'ancienne connexion, évincée, ne repassera
    // pas le nœud `offline` derrière nous.
    let registration = context.registry.register(node.id);
    let connection = registration.connection;
    match context.presence.online(node.id, &host).await {
        Ok(true) => {}
        // Révoqué ou supprimé entre l'identification et le `hello`.
        Ok(false) => {
            context.registry.unregister(node.id, connection);
            revoked(ws, DisconnectReason::Revoked).await;
            return None;
        }
        Err(error) => {
            tracing::error!(%error, "failed to mark node online");
            context.registry.unregister(node.id, connection);
            return None;
        }
    }

    // Lue avant l'acquittement : aucune requête de la session ne chevauche celles qui suivent
    // la connexion.
    let assigned = match context.servers.assigned(node.id).await {
        Ok(assigned) => assigned,
        Err(error) => {
            tracing::error!(%error, "failed to list the servers of the node");
            Vec::new()
        }
    };
    let running_backups = match context.servers.running_backups(node.id).await {
        Ok(running) => running,
        Err(error) => {
            tracing::error!(%error, "failed to list the running backups of the node");
            Vec::new()
        }
    };

    // Rétention de l'historique des fichiers (ADR-0049), lue avec le reste avant l'acquittement.
    let file_history = {
        let mut conn = match context.presence.pool().acquire().await {
            Ok(conn) => Some(conn),
            Err(error) => {
                tracing::error!(%error, "file history policy not read");
                None
            }
        };
        match conn.as_mut() {
            Some(conn) => crate::settings::infra::SettingsRepository::file_history(conn)
                .await
                .map_err(|error| tracing::error!(%error, "file history policy not read"))
                .ok()
                .map(|(days, max_mb)| {
                    crate::servers::application::files::history_policy(days, max_mb)
                }),
            None => None,
        }
    };
    let ack = Envelope::reply(
        envelope.id,
        ApiMessage::HelloAck(HelloAck {
            node_id: node.id.as_uuid(),
            heartbeat_interval_secs: context.heartbeat_interval_secs,
            heartbeat_timeout_secs: context.heartbeat_timeout_secs,
            sftp: context.sftp.is_some(),
            file_history,
        }),
    );
    if let Err(error) = send(ws, &ack).await {
        // La boucle de service constatera la fermeture et repassera le nœud `offline`.
        tracing::debug!(%error, "failed to send hello.ack");
    }
    Some((registration, (assigned, running_backups)))
}

/// Traite les messages jusqu'à la fin de la session et en retourne la raison.
async fn serve(
    ws: &mut WebSocket,
    node: &Node,
    context: &LinkContext,
    mut evicted: oneshot::Receiver<Eviction>,
    mut commands: mpsc::Receiver<Outgoing>,
    shutdown: &CancellationToken,
) -> &'static str {
    // Commandes envoyées, en attente de leur `command.result`.
    let mut pending: HashMap<MessageId, oneshot::Sender<DaemonMessage>> = HashMap::new();
    // Écritures des états en base hors de la boucle : une latence Postgres ne gèle ni la console
    // ni les commandes de ce nœud. La tâche s'arrête avec la session (émetteur lâché).
    let (states, mut queued_states) = mpsc::channel::<ServerStateReport>(STATE_QUEUE);
    tokio::spawn({
        let servers = Arc::clone(&context.servers);
        let node_id = node.id;
        async move {
            while let Some(report) = queued_states.recv().await {
                servers.record_state(node_id, report).await;
            }
        }
    });
    // Réponses aux requêtes du nœud (`sftp.*`), calculées hors de la boucle (base de données).
    let (replies, mut ready_replies) = mpsc::channel::<Envelope<ApiMessage>>(REPLY_QUEUE);
    let mut flood_window = Instant::now();
    let mut messages_in_window: u32 = 0;
    let heartbeat_timeout = std::time::Duration::from_secs(context.heartbeat_timeout_secs);
    let deadline = sleep(heartbeat_timeout);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            () = shutdown.cancelled() => return "api shutting down",
            eviction = &mut evicted => match eviction {
                Ok(Eviction::Disconnected(reason)) => {
                    revoked(ws, reason).await;
                    return reason.as_str();
                }
                Ok(Eviction::Replaced) | Err(_) => return "replaced by a newer connection",
            },
            () = &mut deadline => return "heartbeat timeout",
            Some(reply) = ready_replies.recv() => {
                if let Err(error) = send(ws, &reply).await {
                    tracing::debug!(%error, "failed to send reply");
                    return "connection closed";
                }
            }
            Some(outgoing) = commands.recv() => {
                let envelope = Envelope::new(outgoing.message);
                match send(ws, &envelope).await {
                    Ok(()) => {
                        // Requêtes expirées côté appelant (récepteur parti) : oubliées ici aussi,
                        // sinon la carte grossit à chaque commande restée sans réponse.
                        pending.retain(|_, reply| !reply.is_closed());
                        pending.insert(envelope.id, outgoing.reply);
                    }
                    // Trop gros pour un message : l'appelant reçoit un échec, le lien reste.
                    Err(SendError::Codec(error)) => {
                        tracing::warn!(%error, "command not sent");
                    }
                    Err(SendError::WebSocket(error)) => {
                        tracing::debug!(%error, "failed to send command");
                        return "connection closed";
                    }
                }
            }
            incoming = next_text(ws) => match incoming {
                Incoming::Text(_) if {
                    if flood_window.elapsed() >= FLOOD_WINDOW {
                        flood_window = Instant::now();
                        messages_in_window = 0;
                    }
                    messages_in_window += 1;
                    messages_in_window > MAX_MESSAGES_PER_WINDOW
                } => {
                    tracing::warn!("node floods the link, closing its session");
                    reject(ws, None, ErrorCode::InvalidMessage, "too many messages").await;
                    return "message flood";
                }
                Incoming::Closed => return "connection closed",
                Incoming::Binary => {
                    reject(ws, None, ErrorCode::InvalidMessage, "binary frames are not supported").await;
                    return "invalid message";
                }
                Incoming::Text(text) => match decode::<DaemonMessage>(text.as_str()) {
                    // Réponse à une requête en attente, quelle qu'elle soit.
                    Ok(Envelope { reply_to: Some(request), message, .. }) if pending.contains_key(&request) => {
                        if let Some(reply) = pending.remove(&request) {
                            let _ = reply.send(message);
                        }
                    }
                    Ok(Envelope { message: DaemonMessage::Heartbeat(_), .. }) => {
                        match context.presence.heartbeat(node.id).await {
                            Ok(LinkStanding::Active) => {
                                deadline.as_mut().reset(Instant::now() + heartbeat_timeout);
                                pending.retain(|_, reply| !reply.is_closed());
                            }
                            Ok(LinkStanding::Revoked) => {
                                revoked(ws, DisconnectReason::Revoked).await;
                                return DisconnectReason::Revoked.as_str();
                            }
                            Ok(LinkStanding::Deleted) => {
                                revoked(ws, DisconnectReason::Deleted).await;
                                return DisconnectReason::Deleted.as_str();
                            }
                            Ok(LinkStanding::Inactive) => return "node no longer online",
                            Err(error) => {
                                tracing::error!(%error, "failed to record heartbeat");
                                return "database error";
                            }
                        }
                    }
                    // Réponse à `server.sync`, ou à une requête dont l'appelant est parti.
                    Ok(Envelope { message: DaemonMessage::CommandResult(result), .. }) => {
                        tracing::debug!(?result, "unsolicited command.result");
                    }
                    Ok(Envelope { message: DaemonMessage::FileListing(_) | DaemonMessage::FileContent(_)
                        | DaemonMessage::FileVersions(_) | DaemonMessage::DeletedFiles(_)
                        | DaemonMessage::BackupListing(_) | DaemonMessage::BackupChunk(_)
                        | DaemonMessage::ActionResult(_), .. }) => {
                        tracing::debug!("unsolicited reply");
                    }
                    Ok(Envelope { message: DaemonMessage::BackupDone(done), .. }) => {
                        context.servers.backup_done(node.id, done);
                    }
                    Ok(Envelope { message: DaemonMessage::ContentDone(done), .. }) => {
                        context.servers.content_done(node.id, done);
                    }
                    Ok(Envelope { message: DaemonMessage::ServerState(report), .. }) => {
                        if states.send(report).await.is_err() {
                            return "state writer stopped";
                        }
                    }
                    Ok(Envelope { message: DaemonMessage::ConsoleLine(output), .. }) => {
                        context.servers.console(node.id, output);
                    }
                    Ok(Envelope { message: DaemonMessage::ServerStats(stats), .. }) => {
                        context.servers.stats(node.id, stats);
                    }
                    Ok(Envelope { message: DaemonMessage::ServerDisk(disk), .. }) => {
                        context.servers.disk_usage(node.id, disk);
                    }
                    Ok(Envelope { message: DaemonMessage::ServerMetrics(metrics), .. }) => {
                        context.servers.metrics(node.id, metrics);
                    }
                    Ok(Envelope { id, message: message @ (DaemonMessage::SftpAuth(_)
                        | DaemonMessage::SftpLease(_) | DaemonMessage::SftpAudit(_)), .. }) => {
                        answer_sftp(context, node, id, message, replies.clone());
                    }
                    Ok(Envelope { id, message: DaemonMessage::Hello(_), .. }) => {
                        reject(ws, Some(id), ErrorCode::UnexpectedMessage, "hello already received").await;
                    }
                    Err(error) => {
                        reject(ws, None, ErrorCode::InvalidMessage, &error.to_string()).await;
                        return "invalid message";
                    }
                },
            },
        }
    }
}

/// Requête SFTP du nœud (ADR-0047), traitée dans sa tâche : la réponse revient par `replies`.
/// Sans SFTP sur cette API (tests du lien), ou en cas d'erreur interne, la réponse est un
/// `error` : le nœud refuse alors la connexion SFTP, le lien reste.
fn answer_sftp(
    context: &LinkContext,
    node: &Node,
    request: MessageId,
    message: DaemonMessage,
    replies: mpsc::Sender<Envelope<ApiMessage>>,
) {
    let sftp = context.sftp.clone();
    let node_id = node.id.as_uuid();
    tokio::spawn(async move {
        let outcome = match (sftp, message) {
            (None, _) => Err("sftp is not served by this api".to_owned()),
            (Some(sftp), DaemonMessage::SftpAuth(auth)) => {
                crate::sftp::application::authenticate(&sftp, node_id, auth)
                    .await
                    .map(ApiMessage::SftpGrant)
                    .map_err(|e| e.to_string())
            }
            (Some(sftp), DaemonMessage::SftpLease(lease)) => {
                crate::sftp::application::renew(&sftp, node_id, lease)
                    .await
                    .map(ApiMessage::SftpLeases)
                    .map_err(|e| e.to_string())
            }
            (Some(sftp), DaemonMessage::SftpAudit(audit)) => {
                crate::sftp::application::record_events(&sftp, node_id, audit)
                    .await
                    .map(ApiMessage::SftpAuditAck)
                    .map_err(|e| e.to_string())
            }
            (Some(_), _) => Err("not an sftp request".to_owned()),
        };
        let reply = outcome.unwrap_or_else(|error| {
            tracing::error!(%error, "sftp request failed");
            ApiMessage::Error(ProtocolError {
                code: ErrorCode::Unavailable,
                message: "sftp request failed".to_owned(),
            })
        });
        // Session fermée entre-temps : le nœud refera sa requête à la reconnexion.
        let _ = replies.send(Envelope::reply(request, reply)).await;
    });
}

/// Empreinte de clé d'hôte annoncée par le nœud : `SHA256:` puis du base64, rien d'autre (elle
/// est affichée telle quelle dans le panel).
fn valid_host_key(fingerprint: &str) -> bool {
    fingerprint.strip_prefix("SHA256:").is_some_and(|rest| {
        !rest.is_empty()
            && rest.len() <= 64
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    })
}

enum Incoming {
    Text(Utf8Bytes),
    Binary,
    Closed,
}

/// Prochain message applicatif. Les pings reçoivent leur pong automatiquement.
async fn next_text(ws: &mut WebSocket) -> Incoming {
    while let Some(frame) = ws.next().await {
        match frame {
            Ok(Message::Text(text)) => return Incoming::Text(text),
            Ok(Message::Binary(_)) => return Incoming::Binary,
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(Message::Close(_)) => return Incoming::Closed,
            Err(error) => {
                tracing::debug!(%error, "websocket read failed");
                return Incoming::Closed;
            }
        }
    }
    Incoming::Closed
}

#[derive(Debug, thiserror::Error)]
enum SendError {
    #[error(transparent)]
    Codec(#[from] CodecError),
    #[error(transparent)]
    WebSocket(#[from] axum::Error),
}

async fn send(ws: &mut WebSocket, envelope: &Envelope<ApiMessage>) -> Result<(), SendError> {
    let text = encode(envelope)?;
    ws.send(Message::Text(text.into())).await?;
    Ok(())
}

async fn close(ws: &mut WebSocket) {
    let _ = ws.send(Message::Close(None)).await;
}

/// Envoie un `error` au daemon. La fermeture éventuelle revient à l'appelant.
async fn reject(ws: &mut WebSocket, reply_to: Option<MessageId>, code: ErrorCode, message: &str) {
    tracing::warn!(?code, message, "rejecting daemon message");
    let error = ApiMessage::Error(ProtocolError {
        code,
        message: message.to_owned(),
    });
    let envelope = match reply_to {
        Some(request) => Envelope::reply(request, error),
        None => Envelope::new(error),
    };
    if let Err(error) = send(ws, &envelope).await {
        tracing::debug!(%error, "failed to send error to daemon");
    }
}

/// Le nœud ne doit plus se reconnecter : il en est informé avant la fermeture.
async fn revoked(ws: &mut WebSocket, reason: DisconnectReason) {
    reject(ws, None, ErrorCode::NodeRevoked, reason.as_str()).await;
}
