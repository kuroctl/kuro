//! Messages SFTP (ADR-0047) : requêtes du daemon, réponses de l'API, champs ajoutés à `hello`
//! et `hello.ack` sans casser les versions précédentes.

use kuro_protocol::api::HelloAck;
use kuro_protocol::api::sftp::{SftpAuditAck, SftpDenial, SftpGrant, SftpLeaseResult, SftpLeases};
use kuro_protocol::daemon::sftp::{
    CloseReason, SessionSummary, SftpAudit, SftpAuth, SftpCredential, SftpCredentialRef, SftpEvent,
    SftpEventKind, SftpLease, SftpLeaseRequest, SftpPrincipal,
};
use kuro_protocol::daemon::{Hello, HostInfo, SftpEndpoint};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SESSION: &str = "0192f3a4-1111-7000-8000-000000000001";
const SERVER: &str = "0192f3a4-2222-7000-8000-000000000002";
const ADMIN: &str = "0192f3a4-3333-7000-8000-000000000003";
const KEY: &str = "0192f3a4-4444-7000-8000-000000000004";

fn uuid(text: &str) -> Uuid {
    Uuid::parse_str(text).unwrap()
}

#[test]
fn auth_by_public_key_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::SftpAuth(SftpAuth {
            session_id: uuid(SESSION),
            username: "a1b2c3d4.k7f9x2mq".to_owned(),
            credential: SftpCredential::Publickey {
                algorithm: "ssh-ed25519".to_owned(),
                key: "AAAAC3NzaC1lZDI1NTE5AAAAIA==".to_owned(),
            },
            client_ip: "203.0.113.7".to_owned(),
            client_version: "SSH-2.0-FileZilla_3.69.1".to_owned(),
        }),
    };
    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "sftp.auth",
            "payload": {
                "session_id": SESSION,
                "username": "a1b2c3d4.k7f9x2mq",
                "credential": {
                    "method": "publickey",
                    "algorithm": "ssh-ed25519",
                    "key": "AAAAC3NzaC1lZDI1NTE5AAAAIA=="
                },
                "client_ip": "203.0.113.7",
                "client_version": "SSH-2.0-FileZilla_3.69.1"
            }
        }),
    );
}

#[test]
fn password_never_appears_in_debug() {
    let credential = SftpCredential::Password {
        password: "hunter2-secret".to_owned(),
    };
    let debug = format!("{credential:?}");
    assert!(!debug.contains("hunter2"), "{debug}");
    assert!(debug.contains("redacted"));
}

#[test]
fn grant_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: ApiMessage::SftpGrant(SftpGrant::Granted {
            server_id: uuid(SERVER),
            principal: SftpPrincipal::Admin(uuid(ADMIN)),
            credential: SftpCredentialRef::Key(uuid(KEY)),
            write: false,
            lease_secs: 60,
        }),
    };
    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "sftp.grant",
            "payload": {
                "outcome": "granted",
                "server_id": SERVER,
                "principal": { "kind": "admin", "id": ADMIN },
                "credential": { "kind": "key", "id": KEY },
                "write": false,
                "lease_secs": 60
            }
        }),
    );
}

#[test]
fn denial_wire_format_and_unknown_reason() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: ApiMessage::SftpGrant(SftpGrant::Denied {
            reason: SftpDenial::UnknownKey,
        }),
    };
    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "sftp.grant",
            "payload": { "outcome": "denied", "reason": "unknown_key" }
        }),
    );

    // Une raison ajoutée par une API plus récente reste lisible.
    let newer = json!({
        "id": ID,
        "reply_to": REQUEST_ID,
        "type": "sftp.grant",
        "payload": { "outcome": "denied", "reason": "rate_limited" }
    })
    .to_string();
    let decoded = kuro_protocol::decode::<ApiMessage>(&newer).unwrap();
    assert_eq!(
        decoded.message,
        ApiMessage::SftpGrant(SftpGrant::Denied {
            reason: SftpDenial::Other
        })
    );
}

#[test]
fn lease_round_trip() {
    let request = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::SftpLease(SftpLease {
            sessions: vec![SftpLeaseRequest {
                session_id: uuid(SESSION),
                server_id: uuid(SERVER),
                principal: SftpPrincipal::Admin(uuid(ADMIN)),
                credential: SftpCredentialRef::Key(uuid(KEY)),
            }],
        }),
    };
    assert_wire_format(
        &request,
        json!({
            "id": ID,
            "type": "sftp.lease",
            "payload": { "sessions": [{
                "session_id": SESSION,
                "server_id": SERVER,
                "principal": { "kind": "admin", "id": ADMIN },
                "credential": { "kind": "key", "id": KEY }
            }] }
        }),
    );

    let reply = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: ApiMessage::SftpLeases(SftpLeases {
            results: vec![SftpLeaseResult {
                session_id: uuid(SESSION),
                granted: true,
                write: false,
            }],
        }),
    };
    assert_wire_format(
        &reply,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "sftp.leases",
            "payload": { "results": [{ "session_id": SESSION, "granted": true, "write": false }] }
        }),
    );
}

#[test]
fn audit_round_trip() {
    let request = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::SftpAudit(SftpAudit {
            events: vec![
                SftpEvent {
                    event_id: uuid(KEY),
                    occurred_at_ms: 1_762_000_000_000,
                    session_id: uuid(SESSION),
                    server_id: uuid(SERVER),
                    principal: SftpPrincipal::Admin(uuid(ADMIN)),
                    client_ip: "203.0.113.7".to_owned(),
                    client_version: "SSH-2.0-OpenSSH_9.9".to_owned(),
                    event: SftpEventKind::SessionClosed(SessionSummary {
                        duration_secs: 42,
                        reason: CloseReason::Client,
                        bytes_read: 1024,
                        files_read: 2,
                        bytes_written: 0,
                        files_written: 0,
                        denied: 1,
                    }),
                },
                SftpEvent {
                    event_id: uuid(ID),
                    occurred_at_ms: 1_762_000_000_001,
                    session_id: uuid(SESSION),
                    server_id: uuid(SERVER),
                    principal: SftpPrincipal::Admin(uuid(ADMIN)),
                    client_ip: "203.0.113.7".to_owned(),
                    client_version: "SSH-2.0-OpenSSH_9.9".to_owned(),
                    event: SftpEventKind::AccessDenied {
                        permission: "servers.files.write".to_owned(),
                    },
                },
            ],
        }),
    };
    let text = kuro_protocol::encode(&request).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["type"], "sftp.audit");
    assert_eq!(
        value["payload"]["events"][0]["event"]["action"],
        "session.closed"
    );
    assert_eq!(
        value["payload"]["events"][0]["event"]["details"]["reason"],
        "client"
    );
    assert_eq!(
        value["payload"]["events"][1]["event"]["action"],
        "access.denied"
    );
    let decoded = kuro_protocol::decode::<DaemonMessage>(&text).unwrap();
    assert_eq!(decoded, request);

    let ack = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: ApiMessage::SftpAuditAck(SftpAuditAck { recorded: 2 }),
    };
    assert_wire_format(
        &ack,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "sftp.audit.ack",
            "payload": { "recorded": 2 }
        }),
    );
}

#[test]
fn hello_with_and_without_sftp() {
    let mut hello = Hello {
        protocol_version: 1,
        daemon_version: "0.3.0".to_owned(),
        host: HostInfo {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
            cpu_cores: 4,
            memory_bytes: 8,
        },
        sftp: None,
    };
    // Absent : même format qu'avant l'ADR-0047.
    let text = kuro_protocol::encode(&Envelope::new(DaemonMessage::Hello(hello.clone()))).unwrap();
    assert!(!text.contains("sftp"));

    hello.sftp = Some(SftpEndpoint {
        port: 2022,
        host_key_sha256: "SHA256:abc".to_owned(),
    });
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::Hello(hello),
    };
    let text = kuro_protocol::encode(&envelope).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value["payload"]["sftp"],
        json!({ "port": 2022, "host_key_sha256": "SHA256:abc" })
    );
    assert_eq!(
        kuro_protocol::decode::<DaemonMessage>(&text).unwrap(),
        envelope
    );
}

#[test]
fn hello_ack_from_an_older_api_has_no_sftp() {
    let text = json!({
        "id": ID,
        "reply_to": REQUEST_ID,
        "type": "hello.ack",
        "payload": { "node_id": ADMIN, "heartbeat_interval_secs": 15, "heartbeat_timeout_secs": 45 }
    })
    .to_string();
    let decoded = kuro_protocol::decode::<ApiMessage>(&text).unwrap();
    let ApiMessage::HelloAck(HelloAck { sftp, .. }) = decoded.message else {
        panic!("expected hello.ack");
    };
    assert!(!sftp);
}

#[test]
fn write_events_wire_format() {
    let event = |kind: SftpEventKind| SftpEvent {
        event_id: uuid(KEY),
        occurred_at_ms: 1,
        session_id: uuid(SESSION),
        server_id: uuid(SERVER),
        principal: SftpPrincipal::Admin(uuid(ADMIN)),
        client_ip: "203.0.113.7".to_owned(),
        client_version: "SSH-2.0-WinSCP_6.5".to_owned(),
        event: kind,
    };
    let cases = [
        (
            SftpEventKind::FileWritten {
                path: "/server/data/world.zip".to_owned(),
                size: 42,
            },
            json!({ "action": "file.written", "details": { "path": "/server/data/world.zip", "size": 42 } }),
        ),
        (
            SftpEventKind::DirectoryCreated {
                path: "/server/data/mods".to_owned(),
            },
            json!({ "action": "directory.created", "details": { "path": "/server/data/mods" } }),
        ),
        (
            SftpEventKind::FileRenamed {
                from: "/server/data/a".to_owned(),
                to: "/server/data/b".to_owned(),
            },
            json!({ "action": "file.renamed", "details": { "from": "/server/data/a", "to": "/server/data/b" } }),
        ),
        (
            SftpEventKind::FileDeleted {
                path: "/server/data/old".to_owned(),
            },
            json!({ "action": "file.deleted", "details": { "path": "/server/data/old" } }),
        ),
        (
            SftpEventKind::ModeChanged {
                path: "/server/data/start.sh".to_owned(),
                mode: 0o755,
            },
            json!({ "action": "file.mode_changed", "details": { "path": "/server/data/start.sh", "mode": 493 } }),
        ),
    ];
    for (kind, expected) in cases {
        let message = DaemonMessage::SftpAudit(SftpAudit {
            events: vec![event(kind)],
        });
        let text = kuro_protocol::encode(&Envelope::new(message.clone())).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["payload"]["events"][0]["event"], expected);
        assert_eq!(
            kuro_protocol::decode::<DaemonMessage>(&text)
                .unwrap()
                .message,
            message
        );
    }
}
