//! Accès SFTP côté API (ADR-0047) : clés SSH du compte, coordonnées de connexion, et réponses
//! aux requêtes `sftp.*` du nœud (authentification, baux, audit des sessions).

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::link::{recv, send};
use crate::support::platform::{FakeDaemon, Platform};

const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl alice@laptop";
const KEY_BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
const OTHER_BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAbfoTFkZW/rYsHYt9zZUMHKkZg9+f8H2mqpstq8lydU";
const HOST_KEY: &str = "SHA256:3q2+7wAbCdEfGhIjKlMnOpQrStUvWxYz0123456789abc";

/// Requête du nœud et sa réponse (messages sans rapport ignorés).
async fn ask(daemon: &mut FakeDaemon, kind: &str, payload: Value) -> Value {
    let id = Uuid::now_v7().to_string();
    send(
        &mut daemon.ws,
        json!({ "id": id, "type": kind, "payload": payload }),
    )
    .await;
    loop {
        let message = recv(&mut daemon.ws).await.expect("link closed");
        if message["reply_to"] == id {
            return message;
        }
    }
}

fn auth(username: &str, blob: &str) -> Value {
    json!({
        "session_id": Uuid::now_v7(),
        "username": username,
        "credential": { "method": "publickey", "algorithm": "ssh-ed25519", "key": blob },
        "client_ip": "203.0.113.7",
        "client_version": "SSH-2.0-FileZilla_3.69.1",
    })
}

struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    server_id: String,
}

/// Nœud en ligne qui annonce son SFTP, un serveur déployé, SFTP ouvert dans le projet.
async fn setup() -> Setup {
    let platform = Platform::start().await;
    let node = platform.api.enroll_node("node-1").await;
    let mut ws = platform.api.connect(&node).await;
    let hello_id = Uuid::now_v7().to_string();
    let mut hello = crate::support::link::hello(&hello_id, kuro_protocol::PROTOCOL_VERSION);
    hello["payload"]["sftp"] = json!({ "port": 2022, "host_key_sha256": HOST_KEY });
    send(&mut ws, hello).await;
    let ack = recv(&mut ws).await.unwrap();
    assert_eq!(ack["type"], "hello.ack");
    assert_eq!(ack["payload"]["sftp"], true, "{ack}");
    assert_eq!(recv(&mut ws).await.unwrap()["type"], "server.sync");
    let created = platform
        .call(
            Method::POST,
            &format!("/nodes/{}/allocations", node.id),
            Some(json!({ "ports": "25565" })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let mut daemon = FakeDaemon { node, ws };
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let server_id = server["id"].as_str().unwrap().to_owned();
    let enabled = platform
        .call(
            Method::PATCH,
            "/settings",
            Some(json!({ "sftp_enabled": true })),
        )
        .await;
    assert_eq!(enabled.status, StatusCode::OK, "{}", enabled.json);
    Setup {
        platform,
        daemon,
        server_id,
    }
}

impl Setup {
    /// Nom d'utilisateur SFTP d'un compte pour le serveur.
    async fn username(&self, admin_id: Uuid) -> String {
        sqlx::query_scalar(
            "SELECT s.sftp_handle || '.' || a.sftp_handle FROM servers s, admin_users a
             WHERE s.id = $1 AND a.id = $2",
        )
        .bind(Uuid::parse_str(&self.server_id).unwrap())
        .bind(admin_id)
        .fetch_one(&self.platform.api.pool)
        .await
        .unwrap()
    }

    /// Compte avec ces permissions et la clé `KEY` ; retourne son identifiant et son jeton.
    async fn account(&self, email: &str, permissions: &[Permission]) -> (Uuid, String) {
        let account = self.platform.admin.account(email, permissions).await;
        let token = self.platform.admin.login(&account).await.access_token;
        let added = self
            .platform
            .admin
            .call(
                Method::POST,
                "/me/ssh-keys",
                Some(&token),
                Some(json!({ "name": "laptop", "public_key": KEY })),
            )
            .await;
        assert_eq!(added.status, StatusCode::CREATED, "{}", added.json);
        (account.id, token)
    }
}

const SFTP_READ: &[Permission] = &[Permission::ServersFilesSftp, Permission::ServersFilesRead];

#[tokio::test]
async fn ssh_keys_are_added_listed_and_removed_with_their_audit() {
    let platform = Platform::start().await;
    let added = platform
        .call(
            Method::POST,
            "/me/ssh-keys",
            Some(json!({ "name": "  laptop ", "public_key": KEY })),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.json);
    assert_eq!(added.json["name"], "laptop");
    assert_eq!(added.json["algorithm"], "ssh-ed25519");
    let fingerprint = added.json["fingerprint"].as_str().unwrap().to_owned();
    assert!(fingerprint.starts_with("SHA256:"));
    let event = platform.admin.last_audit("iam.self.ssh_key_added").await;
    assert_eq!(event.details["fingerprint"], fingerprint.as_str());
    // Jamais la clé elle-même dans le journal.
    assert!(!event.details.to_string().contains(KEY_BLOB));

    let duplicate = platform
        .call(
            Method::POST,
            "/me/ssh-keys",
            Some(json!({ "name": "again", "public_key": KEY })),
        )
        .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(duplicate.json["code"], "ssh_key_exists");

    let listed = platform.call(Method::GET, "/me/ssh-keys", None).await;
    assert_eq!(listed.json.as_array().unwrap().len(), 1);

    let id = added.json["id"].as_str().unwrap();
    let removed = platform
        .call(Method::DELETE, &format!("/me/ssh-keys/{id}"), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let event = platform.admin.last_audit("iam.self.ssh_key_removed").await;
    assert_eq!(event.details["fingerprint"], fingerprint.as_str());
    let again = platform
        .call(Method::DELETE, &format!("/me/ssh-keys/{id}"), None)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(again.json["code"], "ssh_key_not_found");
}

#[tokio::test]
async fn invalid_and_rsa_keys_are_refused_on_their_field() {
    let platform = Platform::start().await;
    for (key, code) in [
        ("not a key", "ssh_key_invalid"),
        ("ssh-ed25519 AAAA", "ssh_key_invalid"),
        (
            "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQ== me",
            "ssh_key_unsupported",
        ),
    ] {
        let refused = platform
            .call(
                Method::POST,
                "/me/ssh-keys",
                Some(json!({ "name": "k", "public_key": key })),
            )
            .await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY, "{key}");
        assert_eq!(
            refused.json["fields"][0]["field"], "public_key",
            "{}",
            refused.json
        );
        assert_eq!(refused.json["fields"][0]["code"], code);
    }
}

#[tokio::test]
async fn a_key_belongs_to_its_account_only() {
    let platform = Platform::start().await;
    let other = platform.admin.account("bob@example.com", &[]).await;
    let token = platform.admin.login(&other).await.access_token;
    let added = platform
        .call(
            Method::POST,
            "/me/ssh-keys",
            Some(json!({ "name": "root", "public_key": KEY })),
        )
        .await;
    let id = added.json["id"].as_str().unwrap();
    let removed = platform
        .admin
        .call(
            Method::DELETE,
            &format!("/me/ssh-keys/{id}"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NOT_FOUND);
    let listed = platform
        .admin
        .call(Method::GET, "/me/ssh-keys", Some(&token), None)
        .await;
    assert_eq!(listed.json, json!([]));
}

#[tokio::test]
async fn connection_details_name_the_node_its_port_and_host_key() {
    let setup = setup().await;
    let url = format!("/servers/{}/sftp", setup.server_id);
    let details = setup.platform.call(Method::GET, &url, None).await;
    assert_eq!(details.status, StatusCode::OK, "{}", details.json);
    assert_eq!(details.json["enabled"], true);
    assert_eq!(details.json["port"], 2022);
    assert_eq!(details.json["host_key_fingerprint"], HOST_KEY);
    assert_eq!(details.json["node_name"], "node-1");
    let username = details.json["username"].as_str().unwrap();
    assert!(username.contains('.'), "{username}");

    // Sans `servers.files.sftp` : refusé et journalisé.
    let (_, token) = setup
        .account("reader@example.com", &[Permission::ServersFilesRead])
        .await;
    let denied = setup
        .platform
        .admin
        .call(Method::GET, &url, Some(&token), None)
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let event = setup.platform.admin.last_audit("access.denied").await;
    assert_eq!(event.details["permission"], "servers.files.sftp");
}

#[tokio::test]
async fn a_registered_key_with_the_permissions_opens_a_session() {
    let mut setup = setup().await;
    let (admin_id, _) = setup.account("alice@example.com", SFTP_READ).await;
    let username = setup.username(admin_id).await;

    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["type"], "sftp.grant", "{reply}");
    let grant = &reply["payload"];
    assert_eq!(grant["outcome"], "granted", "{reply}");
    assert_eq!(grant["server_id"], setup.server_id.as_str());
    assert_eq!(
        grant["principal"],
        json!({ "kind": "admin", "id": admin_id })
    );
    assert_eq!(grant["write"], false);
    assert_eq!(grant["lease_secs"], 60);

    let event = setup.platform.admin.last_audit("sftp.session.opened").await;
    assert_eq!(event.actor_kind, "admin");
    assert_eq!(event.actor_id, Some(admin_id));
    assert_eq!(event.target_id.as_deref(), Some(setup.server_id.as_str()));
    assert_eq!(event.details["method"], "publickey");
    assert!(
        event.details["key_fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );

    // Un bail renouvelé tant que tout tient ; refusé dès que la clé disparaît.
    let lease = json!({ "sessions": [{
        "session_id": Uuid::now_v7(),
        "server_id": setup.server_id,
        "principal": grant["principal"],
        "credential": grant["credential"],
    }] });
    let renewed = ask(&mut setup.daemon, "sftp.lease", lease.clone()).await;
    assert_eq!(
        renewed["payload"]["results"][0]["granted"], true,
        "{renewed}"
    );
    sqlx::query("DELETE FROM sftp_keys WHERE admin_id = $1")
        .bind(admin_id)
        .execute(&setup.platform.api.pool)
        .await
        .unwrap();
    let refused = ask(&mut setup.daemon, "sftp.lease", lease).await;
    assert_eq!(refused["payload"]["results"][0]["granted"], false);
}

#[tokio::test]
async fn the_write_permission_is_granted_with_the_session() {
    let mut setup = setup().await;
    let (admin_id, _) = setup
        .account(
            "writer@example.com",
            &[
                Permission::ServersFilesSftp,
                Permission::ServersFilesRead,
                Permission::ServersFilesWrite,
            ],
        )
        .await;
    let username = setup.username(admin_id).await;
    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["payload"]["write"], true, "{reply}");
}

#[tokio::test]
async fn refusals_are_journaled_by_reason() {
    let mut setup = setup().await;
    let (admin_id, _) = setup.account("alice@example.com", SFTP_READ).await;
    let username = setup.username(admin_id).await;

    // Clé d'un autre compte (ou inconnue).
    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, OTHER_BLOB)).await;
    assert_eq!(
        reply["payload"],
        json!({ "outcome": "denied", "reason": "unknown_key" })
    );
    let event = setup.platform.admin.last_audit("auth.sftp.failed").await;
    assert_eq!(event.outcome, "failure");
    assert_eq!(event.details["reason"], "unknown_key");
    assert_eq!(event.actor_id, Some(admin_id));

    // Compte inconnu.
    let server_handle = username.split('.').next().unwrap();
    let reply = ask(
        &mut setup.daemon,
        "sftp.auth",
        auth(&format!("{server_handle}.nobody"), KEY_BLOB),
    )
    .await;
    assert_eq!(reply["payload"]["reason"], "unknown_account");

    // Nom mal formé : refusé sans trace.
    let before = setup.platform.admin.audit().await.len();
    let reply = ask(&mut setup.daemon, "sftp.auth", auth("root", KEY_BLOB)).await;
    assert_eq!(reply["payload"]["reason"], "invalid_username");
    assert_eq!(setup.platform.admin.audit().await.len(), before);

    // Mot de passe : pas encore pris en charge (tranche 3).
    let mut password = auth(&username, KEY_BLOB);
    password["credential"] = json!({ "method": "password", "password": "hunter2-secret" });
    let reply = ask(&mut setup.daemon, "sftp.auth", password).await;
    assert_eq!(reply["payload"]["reason"], "bad_password");
    let journal = serde_json::to_string(
        &setup
            .platform
            .admin
            .audit()
            .await
            .into_iter()
            .map(|row| row.details)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!journal.contains("hunter2"));
}

#[tokio::test]
async fn a_missing_permission_is_an_access_denial() {
    let mut setup = setup().await;
    let (admin_id, _) = setup
        .account("reader@example.com", &[Permission::ServersFilesRead])
        .await;
    let username = setup.username(admin_id).await;
    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["payload"]["reason"], "permission_denied");
    let event = setup.platform.admin.last_audit("access.denied").await;
    assert_eq!(event.details["permission"], "servers.files.sftp");
    assert_eq!(event.details["via"], "sftp");
    assert_eq!(event.actor_id, Some(admin_id));
}

#[tokio::test]
async fn sftp_switched_off_or_a_disabled_account_refuses_everything() {
    let mut setup = setup().await;
    let (admin_id, _) = setup.account("alice@example.com", SFTP_READ).await;
    let username = setup.username(admin_id).await;

    setup
        .platform
        .call(
            Method::PATCH,
            "/settings",
            Some(json!({ "sftp_enabled": false })),
        )
        .await;
    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["payload"]["reason"], "disabled");
    let event = setup.platform.admin.last_audit("auth.sftp.failed").await;
    assert_eq!(event.details["reason"], "sftp_disabled");

    setup
        .platform
        .call(
            Method::PATCH,
            "/settings",
            Some(json!({ "sftp_enabled": true })),
        )
        .await;
    sqlx::query("UPDATE admin_users SET disabled_at = now() WHERE id = $1")
        .bind(admin_id)
        .execute(&setup.platform.api.pool)
        .await
        .unwrap();
    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["payload"]["reason"], "unknown_account");
}

#[tokio::test]
async fn a_node_cannot_open_a_session_on_a_server_of_another_node() {
    let mut setup = setup().await;
    let (admin_id, _) = setup.account("alice@example.com", SFTP_READ).await;
    let username = setup.username(admin_id).await;
    let mut other = setup.platform.online_node("node-2", "25566").await;
    let reply = ask(&mut other, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["payload"]["reason"], "unknown_server");
    // Le nœud du serveur, lui, l'obtient.
    let reply = ask(&mut setup.daemon, "sftp.auth", auth(&username, KEY_BLOB)).await;
    assert_eq!(reply["payload"]["outcome"], "granted");
}

fn closed_event(event_id: Uuid, server_id: &str, admin_id: Uuid) -> Value {
    json!({
        "event_id": event_id,
        "occurred_at_ms": 1_762_000_000_000_i64,
        "session_id": Uuid::now_v7(),
        "server_id": server_id,
        "principal": { "kind": "admin", "id": admin_id },
        "client_ip": "203.0.113.7",
        "client_version": "SSH-2.0-OpenSSH_9.9",
        "event": { "action": "session.closed", "details": {
            "duration_secs": 42, "reason": "client", "bytes_read": 1024, "files_read": 2,
            "bytes_written": 0, "files_written": 0, "denied": 1
        } }
    })
}

#[tokio::test]
async fn session_events_are_recorded_once_and_only_for_the_node_s_servers() {
    let mut setup = setup().await;
    let (admin_id, _) = setup.account("alice@example.com", SFTP_READ).await;
    let event_id = Uuid::now_v7();
    let batch = json!({ "events": [closed_event(event_id, &setup.server_id, admin_id)] });

    let ack = ask(&mut setup.daemon, "sftp.audit", batch.clone()).await;
    assert_eq!(ack["type"], "sftp.audit.ack", "{ack}");
    assert_eq!(ack["payload"]["recorded"], 1);
    let event = setup.platform.admin.last_audit("sftp.session.closed").await;
    assert_eq!(event.actor_id, Some(admin_id));
    assert_eq!(event.details["reason"], "client");
    assert_eq!(event.details["bytes_read"], 1024);

    // Renvoyé après une coupure : pas compté deux fois.
    let again = ask(&mut setup.daemon, "sftp.audit", batch).await;
    assert_eq!(again["payload"]["recorded"], 0);
    let closed = setup
        .platform
        .admin
        .audit()
        .await
        .into_iter()
        .filter(|row| row.action == "sftp.session.closed")
        .count();
    assert_eq!(closed, 1);

    // Un autre nœud ne parle pas des serveurs de celui-ci.
    let mut other = setup.platform.online_node("node-2", "25566").await;
    let foreign = json!({ "events": [closed_event(Uuid::now_v7(), &setup.server_id, admin_id)] });
    let ack = ask(&mut other, "sftp.audit", foreign).await;
    assert_eq!(ack["payload"]["recorded"], 0);

    // Refus d'écriture remonté par le nœud.
    let mut denied = closed_event(Uuid::now_v7(), &setup.server_id, admin_id);
    denied["event"] =
        json!({ "action": "access.denied", "details": { "permission": "servers.files.write" } });
    let ack = ask(
        &mut setup.daemon,
        "sftp.audit",
        json!({ "events": [denied] }),
    )
    .await;
    assert_eq!(ack["payload"]["recorded"], 1);
    let event = setup.platform.admin.last_audit("access.denied").await;
    assert_eq!(event.outcome, "denied");
    assert_eq!(event.details["permission"], "servers.files.write");
    assert_eq!(event.details["via"], "sftp");
}

#[tokio::test]
async fn without_dns_nor_entered_address_the_address_seen_on_the_link_is_shown() {
    let setup = setup().await;
    let url = format!("/servers/{}/sftp", setup.server_id);
    let node: Uuid = setup.daemon.node.id.as_uuid();

    // En test, le nœud se relie depuis 127.0.0.1 : boucle locale, jamais montrée.
    let stored: Option<String> = sqlx::query_scalar("SELECT link_address FROM nodes WHERE id = $1")
        .bind(node)
        .fetch_one(&setup.platform.api.pool)
        .await
        .unwrap();
    assert_eq!(stored.as_deref(), Some("127.0.0.1"));
    let details = setup.platform.call(Method::GET, &url, None).await;
    assert_eq!(details.json["host"], Value::Null);
    assert_eq!(details.json["available"], true);

    // Une adresse joignable vue sur le lien sert de repli...
    sqlx::query("UPDATE nodes SET link_address = '203.0.113.10' WHERE id = $1")
        .bind(node)
        .execute(&setup.platform.api.pool)
        .await
        .unwrap();
    let details = setup.platform.call(Method::GET, &url, None).await;
    assert_eq!(details.json["host"], "203.0.113.10");

    // ... mais l'adresse saisie l'emporte.
    sqlx::query("UPDATE nodes SET public_ipv4 = '198.51.100.4' WHERE id = $1")
        .bind(node)
        .execute(&setup.platform.api.pool)
        .await
        .unwrap();
    let details = setup.platform.call(Method::GET, &url, None).await;
    assert_eq!(details.json["host"], "198.51.100.4");
}

#[tokio::test]
async fn sftp_writes_are_journaled_like_the_panel_s_with_their_channel() {
    let mut setup = setup().await;
    let (admin_id, _) = setup.account("alice@example.com", SFTP_READ).await;
    let event = |action: &str, details: Value| {
        let mut event = closed_event(Uuid::now_v7(), &setup.server_id, admin_id);
        event["event"] = json!({ "action": action, "details": details });
        event
    };
    let batch = json!({ "events": [
        event("file.written", json!({ "path": "/server/data/world.zip", "size": 42 })),
        event("directory.created", json!({ "path": "/server/data/mods" })),
        event("file.renamed", json!({ "from": "/server/data/a", "to": "/server/data/b" })),
        event("file.deleted", json!({ "path": "/server/data/old" })),
        event("file.mode_changed", json!({ "path": "/server/data/start.sh", "mode": 0o755 })),
    ] });
    let ack = ask(&mut setup.daemon, "sftp.audit", batch).await;
    assert_eq!(ack["payload"]["recorded"], 5, "{ack}");

    let journal = setup.platform.admin.audit().await;
    let find = |action: &str| {
        journal
            .iter()
            .rev()
            .find(|row| row.action == action)
            .unwrap_or_else(|| panic!("no {action}"))
            .clone()
    };
    let written = find("server.file_written");
    assert_eq!(written.actor_id, Some(admin_id));
    assert_eq!(written.target_id.as_deref(), Some(setup.server_id.as_str()));
    assert_eq!(
        written.details,
        json!({ "path": "/server/data/world.zip", "size": 42, "via": "sftp" })
    );
    assert_eq!(
        find("server.directory_created").details,
        json!({ "path": "/server/data/mods", "via": "sftp" })
    );
    assert_eq!(
        find("server.file_renamed").details,
        json!({ "from": "/server/data/a", "to": "/server/data/b", "via": "sftp" })
    );
    assert_eq!(
        find("server.file_deleted").details,
        json!({ "path": "/server/data/old", "via": "sftp" })
    );
    assert_eq!(
        find("server.file_mode_changed").details,
        json!({ "path": "/server/data/start.sh", "mode": "755", "via": "sftp" })
    );
}
