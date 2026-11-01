//! Historique des fichiers (ADR-0049) : auteur des écritures, versions, lecture, restauration.

use kuro_protocol::api::{
    AuthorKind, FileAuthor, FileHistory, FileRestore, FileVersionRead, FileWrite,
};
use kuro_protocol::daemon::{FileVersion, FileVersions, VersionChannel, VersionReason};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SERVER: &str = "0192f3a4-2222-7000-8000-000000000002";
const ADMIN: &str = "0192f3a4-3333-7000-8000-000000000003";
const VERSION: &str = "0192f3a4-4444-7000-8000-000000000004";

fn uuid(text: &str) -> Uuid {
    Uuid::parse_str(text).unwrap()
}

fn admin() -> FileAuthor {
    FileAuthor {
        kind: AuthorKind::Admin,
        id: Some(uuid(ADMIN)),
    }
}

#[test]
fn a_write_names_its_author() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::FileWrite(FileWrite {
            server_id: uuid(SERVER),
            path: "/server/data/server.properties".to_owned(),
            content: "motd=Kuro".to_owned(),
            if_sha256: None,
            author: Some(admin()),
        }),
    };
    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "file.write", "payload": {
            "server_id": SERVER, "path": "/server/data/server.properties", "content": "motd=Kuro",
            "author": { "kind": "admin", "id": ADMIN }
        } }),
    );
}

#[test]
fn history_requests_wire_format() {
    let cases = [
        (
            ApiMessage::FileHistory(FileHistory {
                server_id: uuid(SERVER),
                path: "/server/data/a.yml".to_owned(),
            }),
            json!({ "id": ID, "type": "file.history", "payload": { "server_id": SERVER, "path": "/server/data/a.yml" } }),
        ),
        (
            ApiMessage::FileVersionRead(FileVersionRead {
                server_id: uuid(SERVER),
                path: "/server/data/a.yml".to_owned(),
                version: uuid(VERSION),
            }),
            json!({ "id": ID, "type": "file.version.read", "payload": {
                "server_id": SERVER, "path": "/server/data/a.yml", "version": VERSION
            } }),
        ),
        (
            ApiMessage::FileRestore(FileRestore {
                server_id: uuid(SERVER),
                path: "/server/data/a.yml".to_owned(),
                version: uuid(VERSION),
                author: Some(admin()),
            }),
            json!({ "id": ID, "type": "file.restore", "payload": {
                "server_id": SERVER, "path": "/server/data/a.yml", "version": VERSION,
                "author": { "kind": "admin", "id": ADMIN }
            } }),
        ),
    ];
    for (message, expected) in cases {
        let envelope = Envelope {
            id: id(ID),
            reply_to: None,
            message,
        };
        assert_wire_format(&envelope, expected);
    }
}

#[test]
fn versions_wire_format_and_unknown_values() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: DaemonMessage::FileVersions(FileVersions {
            versions: vec![FileVersion {
                id: uuid(VERSION),
                at_ms: 1_762_000_000_000,
                size: 12,
                sha256: "ab".to_owned(),
                reason: VersionReason::Write,
                channel: VersionChannel::Sftp,
                author: Some(admin()),
            }],
        }),
    };
    assert_wire_format(
        &envelope,
        json!({ "id": ID, "reply_to": REQUEST_ID, "type": "file.versions", "payload": { "versions": [{
            "id": VERSION, "at_ms": 1_762_000_000_000_i64, "size": 12, "sha256": "ab",
            "reason": "write", "channel": "sftp", "author": { "kind": "admin", "id": ADMIN }
        }] } }),
    );

    // Valeurs ajoutées par un daemon plus récent : lisibles.
    let newer = json!({ "id": ID, "reply_to": REQUEST_ID, "type": "file.versions", "payload": { "versions": [{
        "id": VERSION, "at_ms": 1, "size": 1, "sha256": "ab", "reason": "rename", "channel": "webdav",
        "author": { "kind": "robot" }
    }] } })
    .to_string();
    let DaemonMessage::FileVersions(versions) = kuro_protocol::decode::<DaemonMessage>(&newer)
        .unwrap()
        .message
    else {
        panic!("expected file.versions");
    };
    let version = &versions.versions[0];
    assert_eq!(version.reason, VersionReason::Other);
    assert_eq!(version.channel, VersionChannel::Other);
    assert_eq!(version.author.unwrap().kind, AuthorKind::Other);
}

#[test]
fn retention_and_deleted_files_wire_format() {
    use kuro_protocol::api::{FileHistoryDeleted, HistoryPolicy, NodeSettings};
    use kuro_protocol::daemon::{DeletedFile, DeletedFiles};

    assert_wire_format(
        &Envelope {
            id: id(ID),
            reply_to: None,
            message: ApiMessage::NodeSettings(NodeSettings {
                file_history: HistoryPolicy {
                    days: 7,
                    max_mb: 50,
                },
            }),
        },
        json!({ "id": ID, "type": "node.settings", "payload": { "file_history": { "days": 7, "max_mb": 50 } } }),
    );
    assert_wire_format(
        &Envelope {
            id: id(ID),
            reply_to: None,
            message: ApiMessage::FileHistoryDeleted(FileHistoryDeleted {
                server_id: uuid(SERVER),
                dir: "/server/data".to_owned(),
            }),
        },
        json!({ "id": ID, "type": "file.history.deleted", "payload": { "server_id": SERVER, "dir": "/server/data" } }),
    );
    let version = FileVersion {
        id: uuid(VERSION),
        at_ms: 1,
        size: 2,
        sha256: "ab".to_owned(),
        reason: VersionReason::Delete,
        channel: VersionChannel::Web,
        author: None,
    };
    assert_wire_format(
        &Envelope {
            id: id(ID),
            reply_to: Some(id(REQUEST_ID)),
            message: DaemonMessage::DeletedFiles(DeletedFiles {
                files: vec![DeletedFile {
                    path: "/server/data/old.yml".to_owned(),
                    version,
                }],
            }),
        },
        json!({ "id": ID, "reply_to": REQUEST_ID, "type": "file.deleted_list", "payload": { "files": [{
            "path": "/server/data/old.yml",
            "version": { "id": VERSION, "at_ms": 1, "size": 2, "sha256": "ab", "reason": "delete", "channel": "web" }
        }] } }),
    );
}

#[test]
fn a_hello_ack_carries_the_history_policy_or_nothing() {
    use kuro_protocol::api::HelloAck;
    let text = json!({
        "id": ID, "reply_to": REQUEST_ID, "type": "hello.ack",
        "payload": { "node_id": ADMIN, "heartbeat_interval_secs": 15, "heartbeat_timeout_secs": 45,
            "file_history": { "days": 0, "max_mb": 100 } }
    })
    .to_string();
    let ApiMessage::HelloAck(HelloAck { file_history, .. }) =
        kuro_protocol::decode::<ApiMessage>(&text).unwrap().message
    else {
        panic!("expected hello.ack");
    };
    assert_eq!(file_history.unwrap().days, 0);
}
