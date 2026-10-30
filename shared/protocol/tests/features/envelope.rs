use kuro_protocol::daemon::Heartbeat;
use kuro_protocol::{DaemonMessage, Envelope, decode, encode};
use serde_json::{Value, json};

use crate::support::{ID, id};

#[test]
fn reply_references_request_id_with_its_own_id() {
    let request = Envelope::new(DaemonMessage::Heartbeat(Heartbeat {}));

    let reply = Envelope::reply(request.id, DaemonMessage::Heartbeat(Heartbeat {}));

    assert_eq!(reply.reply_to, Some(request.id));
    assert_ne!(reply.id, request.id);
}

#[test]
fn reply_to_is_omitted_when_absent() {
    let text = encode(&Envelope::new(DaemonMessage::Heartbeat(Heartbeat {}))).unwrap();
    let json: Value = serde_json::from_str(&text).unwrap();

    assert!(json.get("reply_to").is_none());
}

#[test]
fn unknown_fields_are_ignored() {
    let text = json!({
        "id": ID,
        "trace": "added-by-a-newer-version",
        "type": "heartbeat",
        "payload": { "uptime_secs": 42 }
    })
    .to_string();

    let envelope = decode::<DaemonMessage>(&text).unwrap();

    assert_eq!(envelope.id, id(ID));
    assert_eq!(envelope.message, DaemonMessage::Heartbeat(Heartbeat {}));
}

#[test]
fn missing_id_is_rejected() {
    let text = json!({ "type": "heartbeat", "payload": {} }).to_string();

    assert!(decode::<DaemonMessage>(&text).is_err());
}

#[test]
fn malformed_id_is_rejected() {
    let text = json!({ "id": "not-a-uuid", "type": "heartbeat", "payload": {} }).to_string();

    assert!(decode::<DaemonMessage>(&text).is_err());
}

#[test]
fn unknown_type_is_rejected() {
    let text = json!({ "id": ID, "type": "server.explode", "payload": {} }).to_string();

    assert!(decode::<DaemonMessage>(&text).is_err());
}

#[test]
fn missing_payload_is_rejected() {
    let text = json!({ "id": ID, "type": "heartbeat" }).to_string();

    assert!(decode::<DaemonMessage>(&text).is_err());
}

#[test]
fn generated_ids_are_unique() {
    let first = Envelope::new(DaemonMessage::Heartbeat(Heartbeat {}));
    let second = Envelope::new(DaemonMessage::Heartbeat(Heartbeat {}));

    assert_ne!(first.id, second.id);
}
