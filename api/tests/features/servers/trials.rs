//! Déploiement d'essai d'un brouillon du Studio : nœud de test obligatoire, version d'essai
//! cachée du catalogue, un essai par brouillon, suppression à l'expiration, à la publication,
//! nettoyage des versions d'essai, permissions et audit.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::support::platform::{FakeDaemon, Platform};

/// Nœud en ligne marqué « test ».
async fn test_node(platform: &Platform) -> FakeDaemon {
    let daemon = platform.online_node("test-1", "25565-25570").await;
    let marked = platform
        .call(
            Method::PATCH,
            &format!("/nodes/{}", daemon.node.id.as_uuid()),
            Some(json!({ "trials": true })),
        )
        .await;
    assert_eq!(marked.status, StatusCode::OK, "{}", marked.json);
    assert_eq!(marked.json["trials"], true);
    daemon
}

async fn blank_draft(platform: &Platform) -> String {
    let created = platform
        .call(Method::POST, "/studio/drafts", Some(json!({})))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    created.json["id"].as_str().unwrap().to_owned()
}

fn trial_body(daemon: &FakeDaemon) -> Option<Value> {
    Some(json!({ "node_id": daemon.node.id.as_uuid(), "name": "Essai de my-prefab" }))
}

async fn start_trial(platform: &Platform, daemon: &mut FakeDaemon, draft: &str) -> Value {
    let path = format!("/studio/drafts/{draft}/trial");
    let body = trial_body(daemon);
    let (response, command) =
        tokio::join!(platform.call(Method::POST, &path, body), daemon.accept(),);
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    assert_eq!(command["type"], "server.deploy");
    response.json
}

#[tokio::test]
async fn a_trial_runs_only_on_a_test_node_with_a_hidden_version() {
    let platform = Platform::start().await;
    let draft = blank_draft(&platform).await;
    let other = platform.online_node("prod-1", "25600-25601").await;

    let refused = platform
        .call(
            Method::POST,
            &format!("/studio/drafts/{draft}/trial"),
            trial_body(&other),
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert_eq!(refused.json["code"], "node_not_for_trials");

    let mut daemon = test_node(&platform).await;
    let started = start_trial(&platform, &mut daemon, &draft).await;

    let version = started["version"].as_str().unwrap();
    assert!(version.starts_with("0.1.0-trial."), "{version}");
    let server = platform
        .call(
            Method::GET,
            &format!("/servers/{}", started["server_id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(server.json["trial"]["draft_id"], draft);
    assert_eq!(server.json["prefab"]["version"], version);
    let expires = OffsetDateTime::parse(
        server.json["trial"]["expires_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    let left = expires - OffsetDateTime::now_utc();
    assert!(
        left > time::Duration::minutes(119) && left <= time::Duration::hours(2),
        "{left}"
    );
    // Hors du catalogue, mais lisible par le serveur qui l'utilise.
    let catalog = platform.call(Method::GET, "/prefabs", None).await;
    assert!(
        !catalog
            .json
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "my-prefab")
    );
    let detail = platform
        .call(Method::GET, &format!("/prefabs/my-prefab/{version}"), None)
        .await;
    assert_eq!(detail.json["source"], "trial");
    let reread = platform
        .call(Method::GET, &format!("/studio/drafts/{draft}"), None)
        .await;
    assert_eq!(reread.json["trial"]["server_id"], started["server_id"]);
    let event = platform
        .admin
        .last_audit("prefab_draft.trial_started")
        .await;
    assert_eq!(event.details["draft"], draft);
    assert_eq!(event.details["version"], version);
}

#[tokio::test]
async fn starting_again_replaces_the_running_trial() {
    let platform = Platform::start().await;
    let draft = blank_draft(&platform).await;
    let mut daemon = test_node(&platform).await;
    let first = start_trial(&platform, &mut daemon, &draft).await;

    let path = format!("/studio/drafts/{draft}/trial");
    let body = trial_body(&daemon);
    let (response, (delete, deploy)) =
        tokio::join!(platform.call(Method::POST, &path, body), async {
            let delete = daemon.accept().await;
            let deploy = daemon.accept().await;
            (delete, deploy)
        },);

    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    assert_eq!(delete["type"], "server.delete");
    assert_eq!(delete["payload"]["server_id"], first["server_id"]);
    assert_eq!(deploy["type"], "server.deploy");
    let event = platform.admin.last_audit("prefab_draft.trial_ended").await;
    assert_eq!(event.details["reason"], "replaced");
}

#[tokio::test]
async fn an_expired_trial_is_removed_with_its_version() {
    let platform = Platform::start().await;
    let draft = blank_draft(&platform).await;
    let mut daemon = test_node(&platform).await;
    let started = start_trial(&platform, &mut daemon, &draft).await;
    let server = started["server_id"].as_str().unwrap().to_owned();
    let servers = platform.admin.servers.clone();
    let later = OffsetDateTime::now_utc() + time::Duration::hours(3);

    // Pas encore expiré : rien ne bouge.
    kuro_api::studio::trials::sweep(&servers, OffsetDateTime::now_utc())
        .await
        .unwrap();
    let (swept, delete) = tokio::join!(
        kuro_api::studio::trials::sweep(&servers, later),
        daemon.accept()
    );
    swept.unwrap();
    assert_eq!(delete["type"], "server.delete");
    assert_eq!(delete["payload"]["server_id"], server);
    let event = platform.admin.last_audit("prefab_draft.trial_ended").await;
    assert_eq!(event.actor_kind, "system");
    assert_eq!(event.details["reason"], "expired");

    // Le nœud confirme : le serveur disparaît, puis sa version d'essai.
    daemon.report(&server, "deleted").await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while platform
        .call(Method::GET, &format!("/servers/{server}"), None)
        .await
        .status
        != StatusCode::NOT_FOUND
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "server never removed"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    kuro_api::studio::trials::sweep(&servers, later)
        .await
        .unwrap();
    let version = started["version"].as_str().unwrap();
    let gone = platform
        .call(Method::GET, &format!("/prefabs/my-prefab/{version}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn publishing_the_draft_ends_its_trial() {
    let platform = Platform::start().await;
    let draft = blank_draft(&platform).await;
    let mut daemon = test_node(&platform).await;
    let started = start_trial(&platform, &mut daemon, &draft).await;

    let path = format!("/studio/drafts/{draft}/publish");
    let (published, delete) =
        tokio::join!(platform.call(Method::POST, &path, None), daemon.accept());

    assert_eq!(published.status, StatusCode::CREATED, "{}", published.json);
    assert_eq!(published.json["version"], "0.1.0");
    assert_eq!(delete["payload"]["server_id"], started["server_id"]);
    let event = platform.admin.last_audit("prefab_draft.trial_ended").await;
    assert_eq!(event.details["reason"], "published");
}

#[tokio::test]
async fn a_trial_needs_to_manage_prefabs_and_to_create_servers() {
    let platform = Platform::start().await;
    let draft = blank_draft(&platform).await;
    let daemon = test_node(&platform).await;
    let admin = &platform.admin;
    let account = admin
        .account("studio@example.com", &[Permission::PrefabsManage])
        .await;
    let token = admin.login(&account).await.access_token;

    let refused = admin
        .call(
            Method::POST,
            &format!("/studio/drafts/{draft}/trial"),
            Some(&token),
            trial_body(&daemon),
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json["params"]["permission"], "servers.create");
    let marked = platform.admin.last_audit("node.updated").await;
    assert_eq!(marked.details["fields"], json!(["trials"]));
}
