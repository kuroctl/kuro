//! Envoi de fichiers et modpacks (ADR-0020) : relais par morceaux vérifié, `.jar` déposé dans un
//! emplacement, modpack Modrinth (index lu, version du jeu imposée, fichiers serveur seulement).

use std::io::Write;

use axum::http::{Method, StatusCode};
use base64::Engine;
use kuro_protocol::api::ContentHash;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

use crate::support::content::version;
use crate::support::platform::{FakeDaemon, Platform};

const GAME: &str = "1.21.4";

async fn server(platform: &Platform, daemon: &mut FakeDaemon, settings: Value) -> String {
    let (server, _) = platform.deploy_minecraft(daemon, settings).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    id
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decoded(chunks: &[Value]) -> Vec<u8> {
    chunks
        .iter()
        .flat_map(|c| {
            base64::engine::general_purpose::STANDARD
                .decode(c["payload"]["data"].as_str().unwrap())
                .unwrap()
        })
        .collect()
}

#[tokio::test]
async fn an_upload_is_relayed_in_ordered_chunks_with_its_checksum() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(&platform, &mut daemon, json!({})).await;
    let body: Vec<u8> = (0..1_200_000_u32).map(|i| (i % 251) as u8).collect();

    let path = format!("/servers/{id}/files/upload?path=/server/data/world.zip");
    let (response, (start, chunks)) = tokio::join!(
        platform
            .admin
            .call_bytes(Method::PUT, &path, &platform.token, body.clone(), true),
        daemon.accept_upload(),
    );

    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    assert_eq!(start["payload"]["path"], "/server/data/world.zip");
    assert_eq!(start["payload"]["size"], body.len());
    let offsets: Vec<u64> = chunks
        .iter()
        .map(|c| c["payload"]["offset"].as_u64().unwrap())
        .collect();
    assert_eq!(offsets, [0, 524_288, 1_048_576]);
    assert_eq!(decoded(&chunks), body);
    let sha = hex(&Sha256::digest(&body));
    assert_eq!(chunks.last().unwrap()["payload"]["sha256"], sha);
    assert_eq!(response.json["sha256"], sha);
    let event = platform.admin.last_audit("server.file_uploaded").await;
    assert_eq!(event.details["size"], body.len());
}

#[tokio::test]
async fn an_upload_needs_its_length_and_a_writable_path() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(&platform, &mut daemon, json!({})).await;

    let no_length = platform
        .admin
        .call_bytes(
            Method::PUT,
            &format!("/servers/{id}/files/upload?path=/server/data/x"),
            &platform.token,
            vec![1, 2],
            false,
        )
        .await;
    let escape = platform
        .admin
        .call_bytes(
            Method::PUT,
            &format!("/servers/{id}/files/upload?path=/server/data/../x"),
            &platform.token,
            vec![1],
            true,
        )
        .await;

    assert_eq!(no_length.status, StatusCode::LENGTH_REQUIRED);
    assert_eq!(escape.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_jar_dropped_in_a_slot_is_recorded_as_uploaded() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(
        &platform,
        &mut daemon,
        json!({ "loader": "fabric", "version": GAME }),
    )
    .await;
    let jar = b"PK my private mod".to_vec();

    let path = format!("/servers/{id}/content/mods/upload?file_name=mine.jar");
    let (response, (start, _)) = tokio::join!(
        platform
            .admin
            .call_bytes(Method::POST, &path, &platform.token, jar.clone(), true),
        daemon.accept_upload(),
    );

    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    assert_eq!(start["payload"]["path"], "/server/data/mods/mine.jar");
    assert_eq!(response.json["source"], "upload");
    assert_eq!(response.json["state"], "installed");
    assert_eq!(response.json["name"], "mine");
    let again = platform
        .admin
        .call_bytes(Method::POST, &path, &platform.token, jar, true)
        .await;
    assert_eq!(again.json["code"], "file_exists");
}

/// Archive `.mrpack` : un mod serveur, un fichier client seulement, les dépendances données.
fn mrpack(dependencies: Value) -> Vec<u8> {
    let index = json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "1.0.0",
        "name": "Test Pack",
        "files": [
            {
                "path": "mods/lithium.jar",
                "hashes": { "sha1": "x", "sha512": "a".repeat(128) },
                "env": { "client": "required", "server": "required" },
                "downloads": ["https://cdn.modrinth.com/data/x/lithium.jar"],
                "fileSize": 1000
            },
            {
                "path": "mods/sodium.jar",
                "hashes": { "sha512": "b".repeat(128) },
                "env": { "client": "required", "server": "unsupported" },
                "downloads": ["https://cdn.modrinth.com/data/y/sodium.jar"],
                "fileSize": 2000
            }
        ],
        "dependencies": dependencies,
    });
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("modrinth.index.json", options).unwrap();
    writer.write_all(index.to_string().as_bytes()).unwrap();
    writer
        .start_file("overrides/config/pack.toml", options)
        .unwrap();
    writer.write_all(b"a = 1").unwrap();
    writer.finish().unwrap();
    buffer.into_inner()
}

/// Publie un modpack chez le provider simulé ; retourne l'URL de son archive.
fn publish(platform: &Platform, id: &str, archive: &[u8]) -> String {
    let mut pack = version(id, "test-pack", GAME, "fabric", &[]);
    pack.files[0].url =
        format!("https://cdn.modrinth.com/data/test-pack/versions/{id}/pack.mrpack");
    pack.files[0].file_name = "pack.mrpack".to_owned();
    pack.files[0].hash = ContentHash::Sha512(hex(&Sha512::digest(archive)));
    pack.files[0].size = archive.len() as u64;
    let url = pack.files[0].url.clone();
    platform.admin.content.add("Test Pack", pack);
    platform
        .admin
        .content
        .files
        .lock()
        .unwrap()
        .insert(url.clone(), archive.to_vec());
    url
}

#[tokio::test]
async fn a_modpack_installs_its_server_files_and_leaves_with_them() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(
        &platform,
        &mut daemon,
        json!({ "loader": "fabric", "version": GAME }),
    )
    .await;
    let url = publish(
        &platform,
        "pack-1",
        &mrpack(json!({ "minecraft": GAME, "fabric-loader": "0.16.0" })),
    );

    let path = format!("/servers/{id}/content/mods/modpacks");
    let (response, request) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "version_id": "pack-1" }))),
        daemon.accept_content(),
    );

    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    let payload = &request["payload"];
    assert_eq!(payload["root"], "/data");
    assert_eq!(payload["archive"]["url"], url);
    let files: Vec<&str> = payload["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["file_name"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["mods/lithium.jar"]);
    daemon
        .event(
            "content.done",
            json!({ "server_id": id, "operation_id": payload["operation_id"], "status": "ok" }),
        )
        .await;
    let event = platform.admin.last_audit("server.modpack_installed").await;
    assert_eq!(event.details["files"], 1);

    let list = platform
        .call(Method::GET, &format!("/servers/{id}/content/mods"), None)
        .await
        .json["items"]
        .clone();
    let list = list.as_array().unwrap();
    let pack = list.iter().find(|i| i["kind"] == "modpack").unwrap();
    let child = list.iter().find(|i| i["modpack_id"] == pack["id"]).unwrap();
    assert_eq!(child["file_name"], "mods/lithium.jar");

    let alone = platform
        .call(
            Method::DELETE,
            &format!(
                "/servers/{id}/content/mods/{}",
                child["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(alone.json["code"], "content_in_modpack");
    let second = platform
        .call(Method::POST, &path, Some(json!({ "version_id": "pack-1" })))
        .await;
    assert_eq!(second.json["code"], "modpack_installed");

    let delete = format!(
        "/servers/{id}/content/mods/{}",
        pack["id"].as_str().unwrap()
    );
    let (removed, removal) = tokio::join!(
        platform.call(Method::DELETE, &delete, None),
        daemon.accept_content()
    );
    assert_eq!(removed.status, StatusCode::OK, "{}", removed.json);
    assert_eq!(removal["payload"]["root"], "/data");
    assert_eq!(
        removal["payload"]["file_names"],
        json!(["mods/lithium.jar"])
    );
    let after = platform
        .call(Method::GET, &format!("/servers/{id}/content/mods"), None)
        .await;
    assert_eq!(after.json["items"], json!([]));
}

#[tokio::test]
async fn a_modpack_sets_its_game_version_before_installing() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(
        &platform,
        &mut daemon,
        json!({ "loader": "fabric", "version": "1.21.1" }),
    )
    .await;
    publish(
        &platform,
        "pack-1",
        &mrpack(json!({ "minecraft": GAME, "fabric-loader": "0.16.0" })),
    );

    let path = format!("/servers/{id}/content/mods/modpacks");
    let (response, deploy) = tokio::join!(
        platform.call(Method::POST, &path, Some(json!({ "version_id": "pack-1" }))),
        daemon.accept(),
    );

    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    assert_eq!(deploy["type"], "server.deploy");
    // Puis l'installation, une fois le redéploiement accepté.
    let install = daemon.accept_content().await;
    assert_eq!(install["type"], "content.install");
    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.json["settings"]["version"], GAME);
    platform.admin.last_audit("server.settings_updated").await;
}

#[tokio::test]
async fn a_quilt_modpack_installs_on_a_quilt_server() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(
        &platform,
        &mut daemon,
        json!({ "loader": "quilt", "version": GAME }),
    )
    .await;
    publish(
        &platform,
        "quilt-1",
        &mrpack(json!({ "minecraft": GAME, "quilt-loader": "0.26.0" })),
    );

    let path = format!("/servers/{id}/content/mods/modpacks");
    let (response, request) = tokio::join!(
        platform.call(
            Method::POST,
            &path,
            Some(json!({ "version_id": "quilt-1" }))
        ),
        daemon.accept_content(),
    );

    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    assert_eq!(request["type"], "content.install");
}

#[tokio::test]
async fn an_unreadable_or_unsupported_modpack_is_refused() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = server(
        &platform,
        &mut daemon,
        json!({ "loader": "fabric", "version": GAME }),
    )
    .await;
    publish(
        &platform,
        "rift-1",
        &mrpack(json!({ "minecraft": GAME, "rift": "1.0.0" })),
    );
    let url = publish(
        &platform,
        "broken-1",
        &mrpack(json!({ "minecraft": GAME, "fabric-loader": "0.16.0" })),
    );
    platform
        .admin
        .content
        .files
        .lock()
        .unwrap()
        .insert(url, b"tampered".to_vec());
    let path = format!("/servers/{id}/content/mods/modpacks");

    let unknown = platform
        .call(Method::POST, &path, Some(json!({ "version_id": "rift-1" })))
        .await;
    let broken = platform
        .call(
            Method::POST,
            &path,
            Some(json!({ "version_id": "broken-1" })),
        )
        .await;

    assert_eq!(
        unknown.json["code"], "content_not_supported",
        "{}",
        unknown.json
    );
    assert_eq!(broken.json["code"], "invalid_modpack");
    let list = platform
        .call(Method::GET, &format!("/servers/{id}/content/mods"), None)
        .await;
    assert_eq!(list.json["items"], json!([]));
}
