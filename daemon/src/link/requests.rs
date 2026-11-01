//! Requêtes du daemon vers l'API (ADR-0047, `sftp.*`) : envoyées par la session du lien en
//! cours, la réponse revient par `reply_to`. Lien coupé, ou API qui ne les connaît pas : la
//! requête échoue (`Unavailable`) au lieu d'attendre.

use std::time::Duration;

use kuro_protocol::{ApiMessage, DaemonMessage};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

/// Délai d'une requête, attente d'une session comprise.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Requêtes en attente d'envoi.
const QUEUE: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// Pas de lien, ou API sans SFTP : rien n'a été envoyé, ou la session s'est fermée.
    #[error("api unavailable")]
    Unavailable,
    /// L'API a répondu `error`.
    #[error("rejected by api: {0}")]
    Rejected(String),
    #[error("api did not answer in time")]
    Timeout,
}

#[derive(Debug)]
pub struct ApiRequest {
    pub(super) message: DaemonMessage,
    pub(super) reply: oneshot::Sender<Result<ApiMessage, RequestError>>,
    pub(super) deadline: Instant,
}

/// Côté appelant : clonable, utilisable depuis n'importe quelle tâche.
#[derive(Debug, Clone)]
pub struct ApiRequests {
    queue: mpsc::Sender<ApiRequest>,
}

/// Côté lien : lu par la session en cours.
#[derive(Debug)]
pub struct ApiRequestQueue {
    pub(super) queue: mpsc::Receiver<ApiRequest>,
}

impl ApiRequestQueue {
    /// Prochaine requête, et de quoi y répondre : pour une API simulée (tests).
    pub async fn next(
        &mut self,
    ) -> Option<(
        DaemonMessage,
        oneshot::Sender<Result<ApiMessage, RequestError>>,
    )> {
        self.queue
            .recv()
            .await
            .map(|request| (request.message, request.reply))
    }
}

pub fn channel() -> (ApiRequests, ApiRequestQueue) {
    let (queue, receiver) = mpsc::channel(QUEUE);
    (ApiRequests { queue }, ApiRequestQueue { queue: receiver })
}

impl ApiRequests {
    pub async fn request(&self, message: DaemonMessage) -> Result<ApiMessage, RequestError> {
        let (reply, answer) = oneshot::channel();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let request = ApiRequest {
            message,
            reply,
            deadline,
        };
        self.queue
            .try_send(request)
            .map_err(|_| RequestError::Unavailable)?;
        match tokio::time::timeout_at(deadline, answer).await {
            Ok(Ok(outcome)) => outcome,
            // Session fermée avec la requête en vol.
            Ok(Err(_)) => Err(RequestError::Unavailable),
            Err(_) => Err(RequestError::Timeout),
        }
    }
}
