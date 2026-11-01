//! Protocole local entre le daemon et le processus SFTP : trames JSON préfixées par leur
//! longueur (4 octets, gros-boutiste), 1 Mio au plus. Ce contrat ne sort pas du nœud : il
//! n'appartient pas à `shared/protocol`, mais il en reprend les types SFTP.

use kuro_protocol::api::sftp::{SftpDenial, SftpLeaseResult};
use kuro_protocol::daemon::sftp::{
    CloseReason, SftpAuth, SftpCredentialRef, SftpEvent, SftpLeaseRequest, SftpPrincipal,
};
use kuro_protocol::limits::MAX_MESSAGE_BYTES;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

/// Requête du processus SFTP au daemon ; `request` est repris dans la réponse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerMessage {
    Auth {
        request: u64,
        auth: SftpAuth,
    },
    Lease {
        request: u64,
        sessions: Vec<SftpLeaseRequest>,
    },
    Audit {
        request: u64,
        events: Vec<SftpEvent>,
    },
    /// Écriture permise sur ce serveur maintenant ? (déploiement, suspension, quota)
    Check {
        request: u64,
        server_id: Uuid,
    },
}

/// Réponse du daemon, ou fermeture demandée de sa propre initiative (`close`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeMessage {
    Auth {
        request: u64,
        outcome: AuthOutcome,
    },
    /// Baux renouvelés ; une session absente de la liste n'est pas renouvelée.
    Lease {
        request: u64,
        results: Vec<SftpLeaseResult>,
    },
    /// `accepted` : événements écrits dans la file du daemon. Sinon (file pleine), le processus
    /// SFTP les garde et refuse les écritures.
    Audit {
        request: u64,
        accepted: bool,
    },
    /// Réponse à `check` : `None` si l'écriture est permise.
    Check {
        request: u64,
        refusal: Option<WriteRefusal>,
    },
    /// Sessions d'un serveur à fermer (suspendu, supprimé).
    Close {
        server_id: Uuid,
        reason: CloseReason,
    },
}

/// Raison d'un refus d'écriture, rendue au client SFTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRefusal {
    /// Déploiement ou suppression en cours.
    Busy,
    Suspended,
    DiskQuotaExceeded,
    /// Serveur inconnu du nœud, ou daemon indisponible.
    Unavailable,
}

impl WriteRefusal {
    pub fn message(self) -> &'static str {
        match self {
            Self::Busy => "the server is being deployed, try again in a moment",
            Self::Suspended => "the server is suspended",
            Self::DiskQuotaExceeded => "disk quota exceeded",
            Self::Unavailable => "writing is unavailable right now",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AuthOutcome {
    Granted(Box<Grant>),
    Denied { reason: SftpDenial },
}

/// Session accordée par l'API, complétée par le daemon de ce que voit le serveur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    pub server_id: Uuid,
    pub principal: SftpPrincipal,
    pub credential: SftpCredentialRef,
    pub write: bool,
    pub lease_secs: u32,
    pub layout: Vec<ServiceLayout>,
    /// Rétention de l'historique des fichiers en vigueur sur le nœud (ADR-0049).
    #[serde(default)]
    pub history: kuro_protocol::api::HistoryPolicy,
}

/// Service du serveur déployé : ses volumes et son utilisateur, propriétaire des créations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceLayout {
    pub name: String,
    pub volumes: Vec<String>,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame of {0} bytes exceeds the limit")]
    TooLarge(usize),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub async fn write_frame<W, M>(writer: &mut W, message: &M) -> Result<(), FrameError>
where
    W: AsyncWrite + Unpin,
    M: Serialize,
{
    let bytes = serde_json::to_vec(message)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(FrameError::TooLarge(bytes.len()));
    }
    let length = u32::try_from(bytes.len()).map_err(|_| FrameError::TooLarge(bytes.len()))?;
    writer.write_all(&length.to_be_bytes()).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

/// Prochaine trame ; `None` à la fermeture propre du socket.
pub async fn read_frame<R, M>(reader: &mut R) -> Result<Option<M>, FrameError>
where
    R: AsyncRead + Unpin,
    M: DeserializeOwned,
{
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let length = usize::try_from(u32::from_be_bytes(length)).unwrap_or(usize::MAX);
    if length > MAX_MESSAGE_BYTES {
        return Err(FrameError::TooLarge(length));
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes).await?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip_and_oversized_frames_are_refused() {
        let message = BridgeMessage::Audit {
            request: 7,
            accepted: true,
        };
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &message).await.unwrap();
        let mut reader = buffer.as_slice();
        let decoded: Option<BridgeMessage> = read_frame(&mut reader).await.unwrap();
        assert_eq!(decoded, Some(message));
        let end: Option<BridgeMessage> = read_frame(&mut reader).await.unwrap();
        assert_eq!(end, None);

        let huge = u32::try_from(MAX_MESSAGE_BYTES + 1).unwrap().to_be_bytes();
        let mut reader = huge.as_slice();
        let refused: Result<Option<BridgeMessage>, _> = read_frame(&mut reader).await;
        assert!(matches!(refused, Err(FrameError::TooLarge(_))));
    }
}
