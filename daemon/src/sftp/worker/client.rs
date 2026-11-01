//! Côté processus SFTP du socket Unix : requêtes au daemon, et fermetures qu'il demande. Le
//! socket se reconnecte seul ; tant qu'il est coupé, chaque requête échoue aussitôt (une
//! authentification est refusée, un bail n'est pas renouvelé).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use kuro_protocol::daemon::sftp::{CloseReason, SftpAuth, SftpEvent, SftpLeaseRequest};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::sftp::ipc::{BridgeMessage, WorkerMessage, read_frame, write_frame};

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const CALL_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub enum Call {
    Auth(SftpAuth),
    Lease(Vec<SftpLeaseRequest>),
    Audit(Vec<SftpEvent>),
    Check(Uuid),
}

impl Call {
    fn into_message(self, request: u64) -> WorkerMessage {
        match self {
            Self::Auth(auth) => WorkerMessage::Auth { request, auth },
            Self::Lease(sessions) => WorkerMessage::Lease { request, sessions },
            Self::Audit(events) => WorkerMessage::Audit { request, events },
            Self::Check(server_id) => WorkerMessage::Check { request, server_id },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("daemon unavailable")]
pub struct Unavailable;

type Pending = oneshot::Sender<Result<BridgeMessage, Unavailable>>;

#[derive(Debug, Clone)]
pub struct DaemonClient {
    calls: mpsc::Sender<(Call, Pending)>,
    closes: broadcast::Sender<(Uuid, CloseReason)>,
}

impl DaemonClient {
    pub fn spawn(socket: PathBuf, shutdown: CancellationToken) -> Self {
        let (calls, receiver) = mpsc::channel(256);
        let (closes, _) = broadcast::channel(64);
        tokio::spawn(run(socket, receiver, closes.clone(), shutdown));
        Self { calls, closes }
    }

    pub async fn call(&self, call: Call) -> Result<BridgeMessage, Unavailable> {
        let (reply, answer) = oneshot::channel();
        self.calls
            .try_send((call, reply))
            .map_err(|_| Unavailable)?;
        match tokio::time::timeout(CALL_TIMEOUT, answer).await {
            Ok(Ok(outcome)) => outcome,
            _ => Err(Unavailable),
        }
    }

    /// Fermetures demandées par le daemon : `(serveur, raison)`.
    pub fn closes(&self) -> broadcast::Receiver<(Uuid, CloseReason)> {
        self.closes.subscribe()
    }
}

fn request_of(message: &BridgeMessage) -> Option<u64> {
    match message {
        BridgeMessage::Auth { request, .. }
        | BridgeMessage::Lease { request, .. }
        | BridgeMessage::Audit { request, .. }
        | BridgeMessage::Check { request, .. } => Some(*request),
        BridgeMessage::Close { .. } => None,
    }
}

async fn run(
    socket: PathBuf,
    mut calls: mpsc::Receiver<(Call, Pending)>,
    closes: broadcast::Sender<(Uuid, CloseReason)>,
    shutdown: CancellationToken,
) {
    let mut next: u64 = 0;
    loop {
        let stream = tokio::select! {
            () = shutdown.cancelled() => return,
            stream = UnixStream::connect(&socket) => stream,
        };
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, socket = %socket.display(), "daemon socket unreachable");
                // En attendant, les requêtes échouent tout de suite.
                let retry = tokio::time::sleep(RECONNECT_DELAY);
                tokio::pin!(retry);
                loop {
                    tokio::select! {
                        () = shutdown.cancelled() => return,
                        () = &mut retry => break,
                        Some((_, reply)) = calls.recv() => {
                            let _ = reply.send(Err(Unavailable));
                        }
                    }
                }
                continue;
            }
        };
        tracing::info!("connected to the daemon");
        let (mut reader, mut writer) = stream.into_split();
        // Lecture dans sa tâche : `read_frame` n'est pas annulable sans perdre la trame.
        let (frames, mut incoming) = mpsc::channel::<BridgeMessage>(64);
        let read_task = tokio::spawn(async move {
            loop {
                match read_frame::<_, BridgeMessage>(&mut reader).await {
                    Ok(Some(message)) => {
                        if frames.send(message).await.is_err() {
                            return;
                        }
                    }
                    Ok(None) => return,
                    Err(error) => {
                        tracing::warn!(%error, "unreadable frame from the daemon");
                        return;
                    }
                }
            }
        });
        let mut pending: HashMap<u64, Pending> = HashMap::new();
        loop {
            tokio::select! {
                () = shutdown.cancelled() => {
                    read_task.abort();
                    return;
                }
                Some((call, reply)) = calls.recv() => {
                    next += 1;
                    if let Err(error) = write_frame(&mut writer, &call.into_message(next)).await {
                        tracing::warn!(%error, "failed to write to the daemon");
                        let _ = reply.send(Err(Unavailable));
                        break;
                    }
                    pending.retain(|_, waiting| !waiting.is_closed());
                    pending.insert(next, reply);
                }
                message = incoming.recv() => match message {
                    Some(BridgeMessage::Close { server_id, reason }) => {
                        let _ = closes.send((server_id, reason));
                    }
                    Some(message) => {
                        if let Some(waiting) = request_of(&message).and_then(|id| pending.remove(&id)) {
                            let _ = waiting.send(Ok(message));
                        }
                    }
                    None => break,
                },
            }
        }
        read_task.abort();
        tracing::warn!("daemon socket closed");
        // `pending` lâché : les requêtes en vol échouent.
    }
}
