use kuro_protocol::api::HelloAck;
use kuro_protocol::{ApiMessage, Envelope};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const NODE_ID: &str = "6f1c2d3e-4a5b-4c6d-8e7f-001122334455";

#[test]
fn hello_ack_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: ApiMessage::HelloAck(HelloAck {
            node_id: Uuid::parse_str(NODE_ID).unwrap(),
            heartbeat_interval_secs: 15,
            heartbeat_timeout_secs: 45,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "hello.ack",
            "payload": { "node_id": NODE_ID, "heartbeat_interval_secs": 15, "heartbeat_timeout_secs": 45 }
        }),
    );
}
