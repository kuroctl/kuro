//! Encodage JSON des enveloppes, avec la limite de taille appliquée dans les deux sens.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::envelope::Envelope;
use crate::limits::MAX_MESSAGE_BYTES;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("message too large: {size} bytes (max {max})")]
    TooLarge { size: usize, max: usize },
    #[error("malformed message: {0}")]
    Malformed(#[from] serde_json::Error),
}

pub fn encode<M: Serialize>(envelope: &Envelope<M>) -> Result<String, CodecError> {
    let text = serde_json::to_string(envelope)?;
    check_size(text.len())?;
    Ok(text)
}

/// La taille est vérifiée avant toute désérialisation.
pub fn decode<M: DeserializeOwned>(text: &str) -> Result<Envelope<M>, CodecError> {
    check_size(text.len())?;
    Ok(serde_json::from_str(text)?)
}

fn check_size(size: usize) -> Result<(), CodecError> {
    if size > MAX_MESSAGE_BYTES {
        return Err(CodecError::TooLarge {
            size,
            max: MAX_MESSAGE_BYTES,
        });
    }
    Ok(())
}
