//! Contenu des serveurs (ADR-0020) : installation, suppression, activation, fin d'opération.

use kuro_protocol::api::{
    ContentExtract, ContentFile, ContentHash, ContentInstall, ContentRemove, ContentToggle,
    ExtractRoute,
};
use kuro_protocol::daemon::{ContentDone, ContentOutcome, ExtractedPackage};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";
const OPERATION_ID: &str = "0192f3a4-3333-7000-8000-00000000abcd";

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

fn sodium() -> ContentFile {
    ContentFile {
        url: "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar".to_owned(),
        hash: ContentHash::Sha512("ab".repeat(64)),
        size: 1_048_576,
        file_name: "sodium.jar".to_owned(),
    }
}

#[test]
fn content_install_wire_format() {
    let envelope = request(ApiMessage::ContentInstall(ContentInstall {
        server_id: uuid(SERVER_ID),
        operation_id: uuid(OPERATION_ID),
        service: "server".to_owned(),
        dir: "/data/mods".to_owned(),
        files: vec![sodium()],
        root: None,
        archive: None,
        extract: None,
    }));

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "content.install",
            "payload": {
                "server_id": SERVER_ID,
                "operation_id": OPERATION_ID,
                "service": "server",
                "dir": "/data/mods",
                "files": [{
                    "url": "https://cdn.modrinth.com/data/AANobbMI/versions/x/sodium.jar",
                    "hash": { "algo": "sha512", "value": "ab".repeat(64) },
                    "size": 1_048_576,
                    "file_name": "sodium.jar"
                }]
            }
        }),
    );
}

#[test]
fn a_modpack_install_carries_its_archive() {
    let mut archive = sodium();
    archive.file_name = "pack.mrpack".to_owned();
    let envelope = request(ApiMessage::ContentInstall(ContentInstall {
        server_id: uuid(SERVER_ID),
        operation_id: uuid(OPERATION_ID),
        service: "server".to_owned(),
        dir: "/data/mods".to_owned(),
        files: Vec::new(),
        root: Some("/data".to_owned()),
        archive: Some(archive),
        extract: None,
    }));

    let value = serde_json::to_value(&envelope).unwrap();
    assert_eq!(value["payload"]["root"], "/data");
    assert_eq!(value["payload"]["archive"]["file_name"], "pack.mrpack");
}

#[test]
fn a_curseforge_file_is_checked_by_its_sha1() {
    let file = ContentFile {
        url: "https://edge.forgecdn.net/files/1234/567/jei.jar".to_owned(),
        hash: ContentHash::Sha1("0a".repeat(20)),
        size: 2048,
        file_name: "jei.jar".to_owned(),
    };
    let value = serde_json::to_value(&file).unwrap();
    assert_eq!(
        value["hash"],
        json!({ "algo": "sha1", "value": "0a".repeat(20) })
    );
    assert_eq!(serde_json::from_value::<ContentFile>(value).unwrap(), file);
}

#[test]
fn a_hash_must_fit_its_algorithm() {
    assert!(ContentHash::Sha512("ab".repeat(64)).is_well_formed());
    assert!(ContentHash::Sha1("AB".repeat(20)).is_well_formed());
    assert!(!ContentHash::Sha1("ab".repeat(64)).is_well_formed());
    assert!(!ContentHash::Sha512("zz".repeat(64)).is_well_formed());
    assert!(!ContentHash::Sha1(String::new()).is_well_formed());
}

#[test]
fn content_remove_and_toggle_wire_format() {
    assert_wire_format(
        &request(ApiMessage::ContentRemove(ContentRemove {
            server_id: uuid(SERVER_ID),
            service: "server".to_owned(),
            dir: "/data/mods".to_owned(),
            root: None,
            file_names: vec!["sodium.jar".to_owned()],
            dirs: Vec::new(),
        })),
        json!({
            "id": ID,
            "type": "content.remove",
            "payload": {
                "server_id": SERVER_ID,
                "service": "server",
                "dir": "/data/mods",
                "file_names": ["sodium.jar"]
            }
        }),
    );
    assert_wire_format(
        &request(ApiMessage::ContentToggle(ContentToggle {
            server_id: uuid(SERVER_ID),
            service: "server".to_owned(),
            dir: "/data/mods".to_owned(),
            file_name: "sodium.jar".to_owned(),
            enabled: false,
        })),
        json!({
            "id": ID,
            "type": "content.toggle",
            "payload": {
                "server_id": SERVER_ID,
                "service": "server",
                "dir": "/data/mods",
                "file_name": "sodium.jar",
                "enabled": false
            }
        }),
    );
}

#[test]
fn content_done_wire_format() {
    for (outcome, payload) in [
        (ContentOutcome::Ok, json!({ "status": "ok" })),
        (
            ContentOutcome::Failed {
                reason: "checksum mismatch".to_owned(),
            },
            json!({ "status": "failed", "reason": "checksum mismatch" }),
        ),
    ] {
        let envelope = Envelope {
            id: id(ID),
            reply_to: None,
            message: DaemonMessage::ContentDone(ContentDone {
                server_id: uuid(SERVER_ID),
                operation_id: uuid(OPERATION_ID),
                outcome,
                packages: Vec::new(),
            }),
        };
        let mut expected = json!({ "server_id": SERVER_ID, "operation_id": OPERATION_ID });
        expected
            .as_object_mut()
            .unwrap()
            .extend(payload.as_object().unwrap().clone());
        assert_wire_format(
            &envelope,
            json!({ "id": ID, "type": "content.done", "payload": expected }),
        );
    }
}

#[test]
fn an_extracted_install_carries_its_routes_and_reports_its_packages() {
    let install = ContentInstall {
        server_id: uuid(SERVER_ID),
        operation_id: uuid(OPERATION_ID),
        service: "server".to_owned(),
        dir: "/data/addons".to_owned(),
        files: vec![sodium()],
        root: None,
        archive: None,
        extract: Some(ContentExtract {
            describe: "manifest.json".to_owned(),
            routes: vec![ExtractRoute {
                value: "modules.0.type".to_owned(),
                is: "data".to_owned(),
                dir: "/data/behavior_packs".to_owned(),
            }],
        }),
    };
    let wire = serde_json::to_value(&install).unwrap();
    assert_eq!(
        wire["extract"],
        json!({ "describe": "manifest.json", "routes": [
            { "value": "modules.0.type", "is": "data", "dir": "/data/behavior_packs" }
        ] })
    );
    assert_eq!(
        serde_json::from_value::<ContentInstall>(wire).unwrap(),
        install
    );

    let done = ContentDone {
        server_id: uuid(SERVER_ID),
        operation_id: uuid(OPERATION_ID),
        outcome: ContentOutcome::Ok,
        packages: vec![ExtractedPackage {
            file_name: "more-mobs.mcaddon".to_owned(),
            dir: "/data/behavior_packs".to_owned(),
            name: "more-mobs-1".to_owned(),
            description: json!({ "header": { "uuid": "a1b2", "version": [1, 0, 0] } }),
        }],
    };
    let wire = serde_json::to_value(&done).unwrap();
    assert_eq!(wire["status"], "ok");
    assert_eq!(wire["packages"][0]["name"], "more-mobs-1");
    assert_eq!(serde_json::from_value::<ContentDone>(wire).unwrap(), done);

    let remove = ContentRemove {
        server_id: uuid(SERVER_ID),
        service: "server".to_owned(),
        dir: "/data/addons".to_owned(),
        root: None,
        file_names: Vec::new(),
        dirs: vec!["/data/behavior_packs/more-mobs-1".to_owned()],
    };
    let wire = serde_json::to_value(&remove).unwrap();
    assert_eq!(wire["dirs"], json!(["/data/behavior_packs/more-mobs-1"]));
    assert_eq!(
        serde_json::from_value::<ContentRemove>(wire).unwrap(),
        remove
    );
}
