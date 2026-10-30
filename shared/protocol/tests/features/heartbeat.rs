use kuro_protocol::daemon::Heartbeat;
use kuro_protocol::{DaemonMessage, Envelope};
use serde_json::json;

use crate::support::{ID, assert_wire_format, id};

#[test]
fn heartbeat_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::Heartbeat(Heartbeat {}),
    };

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "heartbeat", "payload": {} }),
    );
}
