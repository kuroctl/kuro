//! Compte Steam de l'installation (ADR-0033) : réglé dans les paramètres, mot de passe chiffré et
//! jamais rendu, envoyé seulement aux étapes `steamcmd` qui l'exigent ; sans lui, un prefab qui
//! l'exige n'est ni déployable ni installable.

use std::io::Write;
use std::path::Path;

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use zip::write::SimpleFileOptions;

use crate::support::platform::{FakeDaemon, Platform};

const ACCOUNT: &str = "/settings/steam-account";
const PASSWORD: &str = "Pa ss!word-42";

/// Valheim, dont l'étape `steamcmd` exige le compte de l'installation (comme Arma 3).
fn account_prefab() -> Vec<u8> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/valheim");
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml")).unwrap();
    let start = manifest.find("assets:\n").unwrap();
    let end = manifest.find("services:\n").unwrap();
    let manifest = format!("{}{}", &manifest[..start], &manifest[end..])
        .replace("id: valheim", "id: steam-account-test")
        .replace(
            "steamcmd: { app: 896660, dir: /home/container, validate: true }",
            "steamcmd: { app: 896660, dir: /home/container, validate: true, login: account }",
        );
    assert!(manifest.contains("login: account"));
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for lang in ["en", "fr"] {
        writer
            .start_file(format!("locales/{lang}.json"), options)
            .unwrap();
        writer
            .write_all(&std::fs::read(dir.join(format!("locales/{lang}.json"))).unwrap())
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn import(platform: &Platform) {
    let imported = platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &platform.token,
            account_prefab(),
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
}

fn server(daemon: &FakeDaemon, prefab_id: &str) -> Value {
    json!({
        "name": "Steam",
        "node_id": daemon.node.id.as_uuid(),
        "prefab_id": prefab_id,
        "memory_mb": 4096,
        "cpu_percent": 200,
        "disk_mb": 10240,
        "settings": { "password": "valheim-pass" },
    })
}

async fn set_account(platform: &Platform) {
    let saved = platform
        .call(
            Method::PUT,
            ACCOUNT,
            Some(json!({ "username": "kuro_servers", "password": PASSWORD })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json);
}

/// Étape `steamcmd` d'un message `server.deploy` ou `server.install`.
fn steamcmd(message: &Value) -> &Value {
    &message["payload"]["install"]["steps"][0]["steamcmd"]
}

#[tokio::test]
async fn the_account_is_kept_without_ever_showing_its_password() {
    let platform = Platform::start().await;

    let empty = platform.call(Method::GET, ACCOUNT, None).await;
    assert_eq!(
        empty.json,
        json!({ "configured": false, "username": null, "updated_at": null })
    );

    // Refusés avant tout enregistrement : le mot de passe finit dans un script SteamCMD.
    for (body, field, code) in [
        (
            json!({ "username": "kuro servers", "password": "x" }),
            "username",
            "steam_account_username_invalid",
        ),
        (
            json!({ "username": "kuro_servers", "password": "a\"b" }),
            "password",
            "steam_account_password_invalid",
        ),
        (
            json!({ "username": "kuro_servers", "password": "" }),
            "password",
            "steam_account_password_invalid",
        ),
    ] {
        let refused = platform.call(Method::PUT, ACCOUNT, Some(body)).await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            refused.json["fields"][0]["field"], field,
            "{}",
            refused.json
        );
        assert_eq!(refused.json["fields"][0]["code"], code);
    }

    set_account(&platform).await;
    let shown = platform.call(Method::GET, ACCOUNT, None).await;
    assert_eq!(shown.json["configured"], true);
    assert_eq!(shown.json["username"], "kuro_servers");
    assert!(!shown.json.to_string().contains(PASSWORD));

    // Chiffré en base, absent de l'audit.
    let stored: Vec<u8> = sqlx::query_scalar("SELECT password FROM steam_accounts")
        .fetch_one(&platform.admin.pool)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains(PASSWORD));
    let event = platform.admin.last_audit("steam_account.updated").await;
    assert_eq!(
        event.details,
        json!({ "scope": "installation", "username": "kuro_servers" })
    );

    let removed = platform.call(Method::DELETE, ACCOUNT, None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let event = platform.admin.last_audit("steam_account.removed").await;
    assert_eq!(event.details["username"], "kuro_servers");
    let again = platform.call(Method::DELETE, ACCOUNT, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(again.json["code"], "steam_account_not_configured");
}

#[tokio::test]
async fn the_account_needs_its_own_permission() {
    let platform = Platform::start().await;
    let admin = &platform.admin;
    let account = admin
        .account("settings@example.com", &[Permission::SystemSettingsUpdate])
        .await;
    let token = admin.login(&account).await.access_token;

    let get = admin.call(Method::GET, ACCOUNT, Some(&token), None).await;
    let put = admin
        .call(
            Method::PUT,
            ACCOUNT,
            Some(&token),
            Some(json!({ "username": "kuro_servers", "password": PASSWORD })),
        )
        .await;
    let delete = admin
        .call(Method::DELETE, ACCOUNT, Some(&token), None)
        .await;

    assert_eq!(get.status, StatusCode::FORBIDDEN);
    assert_eq!(put.status, StatusCode::FORBIDDEN);
    assert_eq!(delete.status, StatusCode::FORBIDDEN);
    assert!(!Permission::SystemSteamAccountsManage.agent_grantable());
}

#[tokio::test]
async fn a_prefab_that_needs_the_account_waits_for_it_then_receives_it() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "2456-2460").await;
    import(&platform).await;

    // Sans compte : signalé au catalogue, refusé à la création.
    let version = platform
        .call(Method::GET, "/prefabs/steam-account-test/1.1.0", None)
        .await;
    assert_eq!(version.json["deployable"], false, "{}", version.json);
    assert_eq!(version.json["steam_account_missing"], true);
    let refused = platform
        .call(
            Method::POST,
            "/servers",
            Some(server(&daemon, "steam-account-test")),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["code"], "steam_account_required");

    set_account(&platform).await;
    let version = platform
        .call(Method::GET, "/prefabs/steam-account-test/1.1.0", None)
        .await;
    assert_eq!(version.json["deployable"], true);
    assert_eq!(version.json["steam_account_missing"], false);

    // Le compte part avec la première installation…
    let (created, deploy) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(server(&daemon, "steam-account-test"))
        ),
        daemon.accept(),
    );
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(
        steamcmd(&deploy)["login"],
        json!({ "username": "kuro_servers", "password": PASSWORD })
    );
    let id = created.json["id"].as_str().unwrap().to_owned();
    // …mais jamais dans ce que le panel lit du serveur.
    let shown = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert!(!shown.json.to_string().contains(PASSWORD));

    // …et avec chaque réinstallation.
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    let reinstall = format!("/servers/{id}/reinstall");
    let (accepted, install) = tokio::join!(
        platform.call(
            Method::POST,
            &reinstall,
            Some(json!({ "mode": "reinstall" })),
        ),
        daemon.accept(),
    );
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    assert_eq!(steamcmd(&install)["login"]["username"], "kuro_servers");

    // Compte retiré : la réinstallation est refusée avant d'atteindre le nœud.
    platform.call(Method::DELETE, ACCOUNT, None).await;
    let refused = platform
        .call(
            Method::POST,
            &format!("/servers/{id}/reinstall"),
            Some(json!({ "mode": "reinstall" })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["code"], "steam_account_required");
}

#[tokio::test]
async fn an_anonymous_prefab_never_receives_the_account() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "2456-2460").await;
    set_account(&platform).await;

    let (created, deploy) = tokio::join!(
        platform.call(Method::POST, "/servers", Some(server(&daemon, "valheim"))),
        daemon.accept(),
    );

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert!(steamcmd(&deploy).is_object(), "{deploy}");
    assert_eq!(steamcmd(&deploy).get("login"), None);
    assert!(!deploy.to_string().contains(PASSWORD));
}
