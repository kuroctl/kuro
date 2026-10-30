//! Fichiers d'un serveur (ADR-0015) : relais vers le nœud, refus traduits, écritures auditées
//! sans leur contenu, permissions.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::platform::{FakeDaemon, Platform};

struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    id: String,
}

async fn deployed() -> Setup {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "running").await;
    platform.wait_for_state(&id, "running").await;
    Setup {
        platform,
        daemon,
        id,
    }
}

impl Setup {
    fn url(&self, path: &str) -> String {
        format!("/servers/{}{path}", self.id)
    }

    /// Appel REST pendant que le nœud répond `kind` / `payload` à la requête reçue.
    async fn exchange(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        kind: &str,
        payload: Value,
    ) -> (crate::support::http::TestResponse, Value) {
        let url = self.url(path);
        let (response, request) = tokio::join!(self.platform.call(method, &url, body), async {
            let request = self.daemon.next_file_request().await;
            self.daemon.answer(&request, kind, payload).await;
            request
        });
        (response, request)
    }
}

const OK: &str = "command.result";

fn refused(code: &str) -> Value {
    json!({ "status": "error", "code": code, "message": "x" })
}

#[tokio::test]
async fn a_directory_is_listed_by_the_node() {
    let mut setup = deployed().await;

    let (response, request) = setup
        .exchange(
            Method::GET,
            "/files?path=/server/data",
            None,
            "file.listing",
            json!({ "entries": [
                { "name": "world", "kind": "directory", "size": 4096, "modified_ms": 1000 },
                { "name": "server.properties", "kind": "file", "size": 120, "modified_ms": 2000 }
            ] }),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(request["type"], "file.list");
    assert_eq!(
        request["payload"],
        json!({ "server_id": setup.id, "path": "/server/data" })
    );
    assert_eq!(response.json["path"], "/server/data");
    assert_eq!(response.json["entries"][1]["name"], "server.properties");
    assert_eq!(response.json["entries"][0]["kind"], "directory");
}

#[tokio::test]
async fn a_file_is_read_with_its_fingerprint() {
    let mut setup = deployed().await;

    let (response, request) = setup
        .exchange(
            Method::GET,
            "/files/content?path=/server/data/server.properties",
            None,
            "file.content",
            json!({ "content": "motd=Kuro\n", "size": 10, "modified_ms": 1000, "sha256": "ab12" }),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(request["type"], "file.read");
    assert_eq!(response.json["content"], "motd=Kuro\n");
    assert_eq!(response.json["sha256"], "ab12");
}

#[tokio::test]
async fn a_write_is_forwarded_and_audited_without_its_content() {
    let mut setup = deployed().await;

    let (response, request) = setup
        .exchange(
            Method::PUT,
            "/files/content",
            Some(json!({
                "path": "/server/data/server.properties",
                "content": "rcon.password=hunter2\n",
                "if_sha256": "ab12",
            })),
            OK,
            json!({ "status": "ok" }),
        )
        .await;

    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.json);
    assert_eq!(request["type"], "file.write");
    assert_eq!(request["payload"]["content"], "rcon.password=hunter2\n");
    assert_eq!(request["payload"]["if_sha256"], "ab12");
    let event = setup.platform.admin.last_audit("server.file_written").await;
    assert_eq!(
        event.details,
        json!({ "path": "/server/data/server.properties", "size": 22 })
    );
    assert!(!event.details.to_string().contains("hunter2"));
}

#[tokio::test]
async fn a_stale_write_is_a_conflict() {
    let mut setup = deployed().await;

    let (response, _) = setup
        .exchange(
            Method::PUT,
            "/files/content",
            Some(json!({ "path": "/server/data/a.txt", "content": "x", "if_sha256": "old" })),
            OK,
            refused("conflict"),
        )
        .await;

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "file_conflict");
}

#[tokio::test]
async fn node_refusals_get_stable_codes() {
    let mut setup = deployed().await;
    let cases = [
        ("not_found", StatusCode::NOT_FOUND, "file_not_found"),
        (
            "not_text",
            StatusCode::UNPROCESSABLE_ENTITY,
            "file_not_text",
        ),
        (
            "too_large",
            StatusCode::UNPROCESSABLE_ENTITY,
            "file_too_large",
        ),
        (
            "invalid_path",
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_path",
        ),
    ];

    for (code, status, expected) in cases {
        let (response, _) = setup
            .exchange(
                Method::GET,
                "/files/content?path=/server/data/level.dat",
                None,
                OK,
                refused(code),
            )
            .await;
        assert_eq!(response.status, status, "{code}");
        assert_eq!(response.json["code"], expected, "{code}");
    }
}

#[tokio::test]
async fn directories_renames_and_deletions_are_audited() {
    let mut setup = deployed().await;

    let (created, mkdir) = setup
        .exchange(
            Method::POST,
            "/files/directories",
            Some(json!({ "path": "/server/data/plugins" })),
            OK,
            json!({ "status": "ok" }),
        )
        .await;
    let (renamed, rename) = setup
        .exchange(
            Method::POST,
            "/files/rename",
            Some(json!({ "from": "/server/data/plugins", "to": "/server/data/mods" })),
            OK,
            json!({ "status": "ok" }),
        )
        .await;
    let (exists, _) = setup
        .exchange(
            Method::POST,
            "/files/rename",
            Some(json!({ "from": "/server/data/a", "to": "/server/data/b" })),
            OK,
            refused("already_exists"),
        )
        .await;
    let (deleted, delete) = setup
        .exchange(
            Method::DELETE,
            "/files?path=/server/data/mods",
            None,
            OK,
            json!({ "status": "ok" }),
        )
        .await;

    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(mkdir["type"], "file.mkdir");
    assert_eq!(renamed.status, StatusCode::NO_CONTENT);
    assert_eq!(rename["payload"]["to"], "/server/data/mods");
    assert_eq!(exists.status, StatusCode::CONFLICT);
    assert_eq!(exists.json["code"], "file_exists");
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(delete["type"], "file.delete");

    let admin = &setup.platform.admin;
    assert_eq!(
        admin.last_audit("server.directory_created").await.details,
        json!({ "path": "/server/data/plugins" })
    );
    assert_eq!(
        admin.last_audit("server.file_renamed").await.details,
        json!({ "from": "/server/data/plugins", "to": "/server/data/mods" })
    );
    assert_eq!(
        admin.last_audit("server.file_deleted").await.details,
        json!({ "path": "/server/data/mods" })
    );
}

#[tokio::test]
async fn an_escaping_path_never_reaches_the_node() {
    let setup = deployed().await;

    for path in [
        "/server/data/../../.kuro/spec.json",
        "/server//data",
        "/a\\..\\b",
    ] {
        let encoded = path.replace('\\', "%5C");
        let read = setup
            .platform
            .call(
                Method::GET,
                &setup.url(&format!("/files/content?path={encoded}")),
                None,
            )
            .await;
        assert_eq!(read.status, StatusCode::UNPROCESSABLE_ENTITY, "{path}");
        assert_eq!(read.json["fields"][0]["field"], "path");
        assert_eq!(read.json["fields"][0]["code"], "invalid_path");
    }
    let rename = setup
        .platform
        .call(
            Method::POST,
            &setup.url("/files/rename"),
            Some(json!({ "from": "/server/data/a", "to": "/server/data/../../x" })),
        )
        .await;
    assert_eq!(rename.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(rename.json["fields"][0]["field"], "to");
}

#[tokio::test]
async fn an_oversized_write_is_refused_before_the_node() {
    let setup = deployed().await;

    let response = setup
        .platform
        .call(
            Method::PUT,
            &setup.url("/files/content"),
            Some(json!({ "path": "/server/data/big.txt", "content": "a".repeat(512 * 1024 + 1) })),
        )
        .await;

    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json["code"], "file_too_large");
}

#[tokio::test]
async fn files_cannot_change_during_a_deployment() {
    let setup = deployed().await;
    sqlx::query("UPDATE servers SET state = 'deploying' WHERE id = $1::uuid")
        .bind(&setup.id)
        .execute(&setup.platform.api.pool)
        .await
        .unwrap();

    let response = setup
        .platform
        .call(
            Method::DELETE,
            &setup.url("/files?path=/server/data/world"),
            None,
        )
        .await;

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "server_busy");
}

#[tokio::test]
async fn reading_and_writing_require_their_permissions() {
    let setup = deployed().await;
    let admin = &setup.platform.admin;
    let reader = admin
        .account("reader@example.com", &[Permission::ServersFilesRead])
        .await;
    let nobody = admin
        .account("nobody@example.com", &[Permission::ServersRead])
        .await;
    let reader = admin.login(&reader).await.access_token;
    let nobody = admin.login(&nobody).await.access_token;

    let list = admin
        .call(Method::GET, &setup.url("/files"), Some(&nobody), None)
        .await;
    let write = admin
        .call(
            Method::PUT,
            &setup.url("/files/content"),
            Some(&reader),
            Some(json!({ "path": "/server/data/a.txt", "content": "x" })),
        )
        .await;
    let delete = admin
        .call(
            Method::DELETE,
            &setup.url("/files?path=/server/data/a.txt"),
            Some(&reader),
            None,
        )
        .await;

    assert_eq!(list.status, StatusCode::FORBIDDEN);
    assert_eq!(list.json["params"]["permission"], "servers.files.read");
    assert_eq!(write.status, StatusCode::FORBIDDEN);
    assert_eq!(write.json["params"]["permission"], "servers.files.write");
    assert_eq!(delete.status, StatusCode::FORBIDDEN);
}
