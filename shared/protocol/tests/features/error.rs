use kuro_protocol::api::{ErrorCode, ProtocolError};
use kuro_protocol::{ApiMessage, Envelope, decode};
use serde_json::json;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

#[test]
fn error_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: ApiMessage::Error(ProtocolError {
            code: ErrorCode::UnsupportedProtocolVersion,
            message: "protocol version 2 is not supported".to_owned(),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "error",
            "payload": { "code": "unsupported_protocol_version", "message": "protocol version 2 is not supported" }
        }),
    );
}

#[test]
fn unknown_error_code_from_newer_api_is_tolerated() {
    let text = json!({
        "id": ID,
        "type": "error",
        "payload": { "code": "node_quarantined", "message": "maintenance" }
    })
    .to_string();

    let envelope = decode::<ApiMessage>(&text).unwrap();

    assert!(matches!(
        envelope.message,
        ApiMessage::Error(ProtocolError {
            code: ErrorCode::Unknown,
            ..
        })
    ));
}

#[test]
fn node_revoked_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::Error(ProtocolError {
            code: ErrorCode::NodeRevoked,
            message: "node revoked".to_owned(),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "error",
            "payload": { "code": "node_revoked", "message": "node revoked" }
        }),
    );
}
