//! Fichiers d'un serveur (ADR-0015) : requêtes `file.*`, réponses, règles de chemin partagées.

use kuro_protocol::api::{FileDelete, FileList, FileMkdir, FileRead, FileRename, FileWrite};
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use kuro_protocol::daemon::{FileContent, FileEntry, FileKind, FileListing};
use kuro_protocol::limits::{MAX_PATH_SEGMENT_BYTES, is_valid_name, path_segments};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope, decode};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";

fn server_id() -> Uuid {
    Uuid::parse_str(SERVER_ID).unwrap()
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
fn file_requests_wire_format() {
    let path = "/server/data/server.properties".to_owned();
    let cases = [
        (
            ApiMessage::FileList(FileList {
                server_id: server_id(),
                path: "/server/data".to_owned(),
            }),
            json!({ "type": "file.list", "payload": { "server_id": SERVER_ID, "path": "/server/data" } }),
        ),
        (
            ApiMessage::FileRead(FileRead {
                server_id: server_id(),
                path: path.clone(),
            }),
            json!({ "type": "file.read", "payload": { "server_id": SERVER_ID, "path": path } }),
        ),
        (
            ApiMessage::FileWrite(FileWrite {
                server_id: server_id(),
                path: path.clone(),
                content: "motd=Kuro\n".to_owned(),
                if_sha256: Some("ab12".to_owned()),
                author: None,
            }),
            json!({ "type": "file.write", "payload": {
                "server_id": SERVER_ID, "path": path, "content": "motd=Kuro\n", "if_sha256": "ab12"
            } }),
        ),
        (
            ApiMessage::FileMkdir(FileMkdir {
                server_id: server_id(),
                path: "/server/data/plugins".to_owned(),
            }),
            json!({ "type": "file.mkdir", "payload": { "server_id": SERVER_ID, "path": "/server/data/plugins" } }),
        ),
        (
            ApiMessage::FileRename(FileRename {
                server_id: server_id(),
                from: "/server/data/a.txt".to_owned(),
                to: "/server/data/b.txt".to_owned(),
            }),
            json!({ "type": "file.rename", "payload": {
                "server_id": SERVER_ID, "from": "/server/data/a.txt", "to": "/server/data/b.txt"
            } }),
        ),
        (
            ApiMessage::FileDelete(FileDelete {
                server_id: server_id(),
                path: "/server/data/world".to_owned(),
                author: None,
            }),
            json!({ "type": "file.delete", "payload": { "server_id": SERVER_ID, "path": "/server/data/world" } }),
        ),
    ];
    for (message, mut expected) in cases {
        expected["id"] = json!(ID);
        assert_wire_format(&request(message), expected);
    }
}

#[test]
fn an_unconditional_write_omits_the_fingerprint() {
    let envelope = request(ApiMessage::FileWrite(FileWrite {
        server_id: server_id(),
        path: "/server/data/new.txt".to_owned(),
        content: String::new(),
        if_sha256: None,
        author: None,
    }));

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "file.write", "payload": {
            "server_id": SERVER_ID, "path": "/server/data/new.txt", "content": ""
        } }),
    );
}

#[test]
fn listing_wire_format() {
    let envelope = reply(DaemonMessage::FileListing(FileListing {
        entries: vec![
            FileEntry {
                name: "world".to_owned(),
                kind: FileKind::Directory,
                size: 4096,
                modified_ms: 1_793_042_865_000,
            },
            FileEntry {
                name: "latest.log".to_owned(),
                kind: FileKind::Symlink,
                size: 12,
                modified_ms: 1_793_042_865_000,
            },
        ],
        truncated: false,
    }));

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "reply_to": REQUEST_ID, "type": "file.listing", "payload": { "entries": [
            { "name": "world", "kind": "directory", "size": 4096, "modified_ms": 1_793_042_865_000_i64 },
            { "name": "latest.log", "kind": "symlink", "size": 12, "modified_ms": 1_793_042_865_000_i64 }
        ] } }),
    );
}

#[test]
fn an_unknown_entry_kind_reads_as_other() {
    let text =
        json!({ "id": ID, "reply_to": REQUEST_ID, "type": "file.listing", "payload": { "entries": [
        { "name": "pipe", "kind": "fifo", "size": 0, "modified_ms": 0 }
    ] } })
        .to_string();

    let envelope: Envelope<DaemonMessage> = decode(&text).unwrap();

    let DaemonMessage::FileListing(listing) = envelope.message else {
        panic!("not a listing");
    };
    assert_eq!(listing.entries[0].kind, FileKind::Other);
}

#[test]
fn content_wire_format() {
    let envelope = reply(DaemonMessage::FileContent(FileContent {
        content: "motd=Kuro\n".to_owned(),
        size: 10,
        modified_ms: 1000,
        sha256: "ab12".to_owned(),
    }));

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "reply_to": REQUEST_ID, "type": "file.content", "payload": {
            "content": "motd=Kuro\n", "size": 10, "modified_ms": 1000, "sha256": "ab12"
        } }),
    );
}

#[test]
fn file_error_codes_wire_format() {
    for (code, text) in [
        (CommandErrorCode::InvalidPath, "invalid_path"),
        (CommandErrorCode::NotFound, "not_found"),
        (CommandErrorCode::AlreadyExists, "already_exists"),
        (CommandErrorCode::Conflict, "conflict"),
        (CommandErrorCode::NotText, "not_text"),
        (CommandErrorCode::TooLarge, "too_large"),
    ] {
        let envelope = reply(DaemonMessage::CommandResult(CommandResult::Error {
            code,
            message: "x".to_owned(),
        }));
        assert_wire_format(
            &envelope,
            json!({ "id": ID, "reply_to": REQUEST_ID, "type": "command.result",
                "payload": { "status": "error", "code": text, "message": "x" } }),
        );
    }
}

#[test]
fn a_path_is_split_into_segments() {
    assert_eq!(path_segments("/"), Some(vec![]));
    assert_eq!(path_segments(""), Some(vec![]));
    assert_eq!(
        path_segments("/server/data/world/"),
        Some(vec!["server", "data", "world"])
    );
    assert_eq!(
        path_segments("server/data/é è.txt"),
        Some(vec!["server", "data", "é è.txt"])
    );
}

#[test]
fn escaping_or_ambiguous_paths_are_refused() {
    for path in [
        "/server/data/../../etc/passwd",
        "/server/./data",
        "/server//data",
        "/server/data\\..\\x",
        "/server/data/a\0b",
        "/server/data/a\nb",
        "//",
    ] {
        assert_eq!(path_segments(path), None, "{path:?}");
    }
    let long = "a".repeat(MAX_PATH_SEGMENT_BYTES + 1);
    assert_eq!(path_segments(&format!("/server/{long}")), None);
}

#[test]
fn a_new_name_is_a_single_plain_segment() {
    assert!(is_valid_name("plugins"));
    assert!(is_valid_name(".hidden"));
    assert!(!is_valid_name(".."));
    assert!(!is_valid_name("a/b"));
    assert!(!is_valid_name(""));
}

#[test]
fn an_upload_is_announced_then_sent_in_ordered_chunks() {
    use kuro_protocol::api::{FileUpload, FileUploadChunk};
    use kuro_protocol::limits::{MAX_MESSAGE_BYTES, UPLOAD_CHUNK_BYTES};

    let server_id = uuid::Uuid::now_v7();
    let upload_id = uuid::Uuid::now_v7();
    let start = serde_json::to_value(kuro_protocol::ApiMessage::FileUpload(FileUpload {
        server_id,
        upload_id,
        path: "/server/data/mods/x.jar".to_owned(),
        size: 3,
        author: None,
    }))
    .unwrap();
    assert_eq!(start["type"], "file.upload");
    assert_eq!(start["payload"]["size"], 3);

    let last = serde_json::to_value(kuro_protocol::ApiMessage::FileUploadChunk(
        FileUploadChunk {
            server_id,
            upload_id,
            offset: 0,
            data: "YWJj".to_owned(),
            eof: true,
            sha256: Some("ba7816bf".to_owned()),
        },
    ))
    .unwrap();
    assert_eq!(last["type"], "file.upload.chunk");
    assert_eq!(last["payload"]["eof"], true);
    assert_eq!(last["payload"]["sha256"], "ba7816bf");
    let middle = FileUploadChunk {
        server_id,
        upload_id,
        offset: 0,
        data: String::new(),
        eof: false,
        sha256: None,
    };
    let middle = serde_json::to_value(&middle).unwrap();
    assert!(middle.get("eof").is_none() && middle.get("sha256").is_none());

    // Un morceau plein, en base64 dans son enveloppe, tient dans un message.
    assert!(UPLOAD_CHUNK_BYTES.div_ceil(3) * 4 + 1024 < MAX_MESSAGE_BYTES);
}
