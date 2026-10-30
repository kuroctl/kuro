//! Sauvegardes (ADR-0018) : commandes, fin d'archivage, liste, morceaux, état `restoring`.

use kuro_protocol::api::{
    BackupCreate, BackupDelete, BackupHook, BackupList, BackupRead, BackupRestore, RestoreMode,
};
use kuro_protocol::daemon::server::{ServerState, ServerStateReport};
use kuro_protocol::daemon::{
    BackupChunk, BackupDone, BackupInfo, BackupListing, BackupOutcome, BackupState,
};
use kuro_protocol::limits::{BACKUP_CHUNK_BYTES, MAX_MESSAGE_BYTES};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope, encode};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";
const BACKUP_ID: &str = "0192f3a4-2222-7000-8000-00000000abcd";
const SHA: &str = "324fb9b3d3a59324cb5d96154f4fbc41c8f2d926f9bb2da1702f0230bd1b88d6";

fn uuid(text: &str) -> Uuid {
    Uuid::parse_str(text).unwrap()
}

fn request(message: ApiMessage) -> Envelope<ApiMessage> {
    Envelope {
        id: id(ID),
        reply_to: None,
        message,
    }
}

fn reply(message: DaemonMessage) -> Envelope<DaemonMessage> {
    Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message,
    }
}

#[test]
fn create_wire_format_with_hooks() {
    let envelope = request(ApiMessage::BackupCreate(BackupCreate {
        server_id: uuid(SERVER_ID),
        backup_id: uuid(BACKUP_ID),
        ignore: vec!["server/data/logs/**".to_owned()],
        before: vec![
            BackupHook {
                console: "save-off".to_owned(),
                wait: None,
            },
            BackupHook {
                console: "save-all flush".to_owned(),
                wait: Some("Saved the game".to_owned()),
            },
        ],
        after: vec![BackupHook {
            console: "save-on".to_owned(),
            wait: None,
        }],
    }));

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "backup.create", "payload": {
            "server_id": SERVER_ID, "backup_id": BACKUP_ID,
            "ignore": ["server/data/logs/**"],
            "before": [{ "console": "save-off" }, { "console": "save-all flush", "wait": "Saved the game" }],
            "after": [{ "console": "save-on" }]
        } }),
    );
}

#[test]
fn a_plain_create_omits_empty_lists() {
    let envelope = request(ApiMessage::BackupCreate(BackupCreate {
        server_id: uuid(SERVER_ID),
        backup_id: uuid(BACKUP_ID),
        ignore: Vec::new(),
        before: Vec::new(),
        after: Vec::new(),
    }));

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "backup.create", "payload": { "server_id": SERVER_ID, "backup_id": BACKUP_ID } }),
    );
}

#[test]
fn restore_delete_list_and_read_wire_format() {
    let ids = json!({ "server_id": SERVER_ID, "backup_id": BACKUP_ID });
    for (message, kind, payload) in [
        (
            ApiMessage::BackupRestore(BackupRestore {
                server_id: uuid(SERVER_ID),
                backup_id: uuid(BACKUP_ID),
                mode: RestoreMode::Clean,
            }),
            "backup.restore",
            json!({ "server_id": SERVER_ID, "backup_id": BACKUP_ID, "mode": "clean" }),
        ),
        (
            ApiMessage::BackupDelete(BackupDelete {
                server_id: uuid(SERVER_ID),
                backup_id: uuid(BACKUP_ID),
            }),
            "backup.delete",
            ids.clone(),
        ),
        (
            ApiMessage::BackupList(BackupList {
                server_id: uuid(SERVER_ID),
            }),
            "backup.list",
            json!({ "server_id": SERVER_ID }),
        ),
        (
            ApiMessage::BackupRead(BackupRead {
                server_id: uuid(SERVER_ID),
                backup_id: uuid(BACKUP_ID),
                offset: 524_288,
            }),
            "backup.read",
            json!({ "server_id": SERVER_ID, "backup_id": BACKUP_ID, "offset": 524_288 }),
        ),
    ] {
        assert_wire_format(
            &request(message),
            json!({ "id": ID, "type": kind, "payload": payload }),
        );
    }
}

#[test]
fn done_wire_format_on_success_and_failure() {
    let done = |outcome| Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::BackupDone(BackupDone {
            server_id: uuid(SERVER_ID),
            backup_id: uuid(BACKUP_ID),
            outcome,
        }),
    };

    assert_wire_format(
        &done(BackupOutcome::Ok {
            size: 1024,
            sha256: SHA.to_owned(),
        }),
        json!({ "id": ID, "type": "backup.done", "payload": {
            "server_id": SERVER_ID, "backup_id": BACKUP_ID, "status": "ok", "size": 1024, "sha256": SHA
        } }),
    );
    assert_wire_format(
        &done(BackupOutcome::Failed {
            reason: "disk full".to_owned(),
        }),
        json!({ "id": ID, "type": "backup.done", "payload": {
            "server_id": SERVER_ID, "backup_id": BACKUP_ID, "status": "failed", "reason": "disk full"
        } }),
    );
}

#[test]
fn listing_wire_format() {
    let envelope = reply(DaemonMessage::BackupListing(BackupListing {
        backups: vec![
            BackupInfo {
                backup_id: uuid(BACKUP_ID),
                state: BackupState::Ready {
                    size: 1024,
                    sha256: SHA.to_owned(),
                },
            },
            BackupInfo {
                backup_id: uuid(SERVER_ID),
                state: BackupState::Creating,
            },
        ],
    }));

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "reply_to": REQUEST_ID, "type": "backup.listing", "payload": { "backups": [
            { "backup_id": BACKUP_ID, "state": "ready", "size": 1024, "sha256": SHA },
            { "backup_id": SERVER_ID, "state": "creating" }
        ] } }),
    );
}

#[test]
fn a_full_chunk_fits_in_one_message() {
    let data = "A".repeat(BACKUP_CHUNK_BYTES.div_ceil(3) * 4);
    let envelope = reply(DaemonMessage::BackupChunk(BackupChunk { data, eof: false }));

    let text = encode(&envelope).unwrap();

    assert!(text.len() < MAX_MESSAGE_BYTES);
}

#[test]
fn restoring_state_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ServerState(ServerStateReport {
            server_id: uuid(SERVER_ID),
            state: ServerState::Restoring,
            reason: None,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "server.state", "payload": { "server_id": SERVER_ID, "state": "restoring" } }),
    );
}
