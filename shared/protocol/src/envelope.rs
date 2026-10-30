//! Enveloppe commune à tous les messages.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifiant unique d'un message (UUID v7, ordonné dans le temps).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageId(Uuid);

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Uuid> for MessageId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Message `M` (un [`crate::DaemonMessage`] ou un [`crate::ApiMessage`]) et ses métadonnées.
///
/// `M` porte `type` et `payload` ; l'enveloppe ajoute `id` et, pour une réponse, `reply_to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<M> {
    pub id: MessageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageId>,
    #[serde(flatten)]
    pub message: M,
}

impl<M> Envelope<M> {
    pub fn new(message: M) -> Self {
        Self {
            id: MessageId::new(),
            reply_to: None,
            message,
        }
    }

    /// Réponse au message `request`.
    pub fn reply(request: MessageId, message: M) -> Self {
        Self {
            id: MessageId::new(),
            reply_to: Some(request),
            message,
        }
    }
}
