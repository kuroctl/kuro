//! Un message ne se décode que dans son sens de circulation.

use kuro_protocol::api::HelloAck;
use kuro_protocol::daemon::Heartbeat;
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope, decode, encode};
use uuid::Uuid;

#[test]
fn api_message_cannot_be_decoded_as_daemon_message() {
    let text = encode(&Envelope::new(ApiMessage::HelloAck(HelloAck {
        node_id: Uuid::nil(),
        heartbeat_interval_secs: 15,
        heartbeat_timeout_secs: 45,
    })))
    .unwrap();

    assert!(decode::<DaemonMessage>(&text).is_err());
}

#[test]
fn daemon_message_cannot_be_decoded_as_api_message() {
    let text = encode(&Envelope::new(DaemonMessage::Heartbeat(Heartbeat {}))).unwrap();

    assert!(decode::<ApiMessage>(&text).is_err());
}
