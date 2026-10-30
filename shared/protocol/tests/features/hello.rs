use kuro_protocol::daemon::{Hello, HostInfo};
use kuro_protocol::{DaemonMessage, Envelope};
use serde_json::json;

use crate::support::{ID, assert_wire_format, id};

#[test]
fn hello_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::Hello(Hello {
            protocol_version: 1,
            daemon_version: "0.1.0".to_owned(),
            host: HostInfo {
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                cpu_cores: 8,
                memory_bytes: 17_179_869_184,
            },
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "hello",
            "payload": {
                "protocol_version": 1,
                "daemon_version": "0.1.0",
                "host": { "os": "linux", "arch": "x86_64", "cpu_cores": 8, "memory_bytes": 17_179_869_184_u64 }
            }
        }),
    );
}

#[test]
fn hello_without_host_is_rejected() {
    let text = json!({
        "id": ID,
        "type": "hello",
        "payload": { "protocol_version": 1, "daemon_version": "0.1.0" }
    })
    .to_string();

    assert!(kuro_protocol::decode::<DaemonMessage>(&text).is_err());
}
