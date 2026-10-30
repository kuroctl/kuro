use kuro_protocol::api::{ErrorCode, ProtocolError};
use kuro_protocol::limits::MAX_MESSAGE_BYTES;
use kuro_protocol::{ApiMessage, CodecError, DaemonMessage, Envelope, decode, encode};

use crate::support::ID;

#[test]
fn oversized_message_is_rejected_before_parsing() {
    // Volontairement invalide en JSON : seule la taille doit être évaluée.
    let text = "x".repeat(MAX_MESSAGE_BYTES + 1);

    let error = decode::<DaemonMessage>(&text).unwrap_err();

    assert!(matches!(error, CodecError::TooLarge { size, .. } if size == MAX_MESSAGE_BYTES + 1));
}

#[test]
fn oversized_message_is_not_encoded() {
    let envelope = Envelope::new(ApiMessage::Error(ProtocolError {
        code: ErrorCode::InvalidMessage,
        message: "x".repeat(MAX_MESSAGE_BYTES),
    }));

    let error = encode(&envelope).unwrap_err();

    assert!(matches!(error, CodecError::TooLarge { .. }));
}

#[test]
fn message_at_the_limit_is_accepted() {
    let base = format!(r#"{{"id":"{ID}","type":"heartbeat","payload":{{}}}}"#);
    let padding = " ".repeat(MAX_MESSAGE_BYTES - base.len());
    let text = format!("{base}{padding}");
    assert_eq!(text.len(), MAX_MESSAGE_BYTES);

    assert!(decode::<DaemonMessage>(&text).is_ok());
}

#[test]
fn invalid_json_is_malformed() {
    let error = decode::<DaemonMessage>("{not json").unwrap_err();

    assert!(matches!(error, CodecError::Malformed(_)));
}
