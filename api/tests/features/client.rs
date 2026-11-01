//! Espace client (ADR-0051, tranche 3) : un fournisseur d'identité d'extension, le propriétaire
//! qui ne voit et ne pilote que ses serveurs, les refus audités, le serveur suspendu.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::http::{Method, StatusCode};
use futures_util::future::BoxFuture;
use kuro_api::extension::{
    self, Extension, ExtensionError, ExtensionFactory, IdentityError, IdentityProvider,
};
use kuro_api::servers::domain::OwnerRef;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::admin::TestAdmin;
use crate::support::db;
use crate::support::http::TestResponse;
use crate::support::platform::{FakeDaemon, Platform};

const ALICE: &str = "shop:alice";
const BOB: &str = "shop:bob";

/// Fournisseur de test : un jeton par propriétaire, `broken` simule une panne.
struct Tokens(HashMap<&'static str, &'static str>);

impl IdentityProvider for Tokens {
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<Option<OwnerRef>, IdentityError>> {
        Box::pin(async move {
            if token == "broken" {
                return Err(IdentityError("backend down".to_owned()));
            }
            Ok(self.0.get(token).and_then(|owner| OwnerRef::parse(*owner)))
        })
    }
}

fn shop() -> ExtensionFactory {
    ExtensionFactory::new("shop", |_| {
        let tokens = Tokens(HashMap::from([("alice-token", ALICE), ("bob-token", BOB)]));
        Ok(Extension::default().identity(Arc::new(tokens)))
    })
}

async fn client(
    platform: &Platform,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> TestResponse {
    let authorization = token.map(|token| format!("Bearer {token}"));
    let headers: Vec<(&str, &str)> = authorization
        .as_deref()
        .map(|value| vec![("authorization", value)])
        .unwrap_or_default();
    platform
        .admin
        .raw(
            method,
            &format!("/api/client/v1{path}"),
            &headers,
            body.map(|body| ("application/json", body.to_string())),
        )
        .await
}

/// Serveur déployé pour `owner` (ou interne), arrêté.
async fn server_of(platform: &Platform, daemon: &mut FakeDaemon, owner: Value) -> String {
    let (response, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "owner_ref": owner,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    let id = response.json["id"].as_str().unwrap().to_owned();
    daemon.report(&id, "offline").await;
    platform.wait_for_state(&id, "offline").await;
    id
}

#[tokio::test]
async fn without_identity_provider_there_is_no_client_space() {
    let kuro = TestAdmin::start().await;

    let response = kuro
        .raw(
            Method::GET,
            "/api/client/v1/servers",
            &[("authorization", "Bearer x")],
            None,
        )
        .await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_missing_or_unknown_token_is_unauthorized() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;

    for token in [None, Some("nope"), Some("")] {
        let response = client(&platform, Method::GET, "/servers", token, None).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED, "{token:?}");
        assert_eq!(response.json["code"], "unauthorized");
    }
}

#[tokio::test]
async fn an_admin_token_does_not_open_the_client_space() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;

    let token = platform.token.clone();
    let response = client(&platform, Method::GET, "/servers", Some(&token), None).await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_failing_provider_is_an_internal_error() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;

    let response = client(&platform, Method::GET, "/servers", Some("broken"), None).await;

    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn an_owner_lists_only_their_servers() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    server_of(&platform, &mut daemon, json!(BOB)).await;
    server_of(&platform, &mut daemon, Value::Null).await;

    let response = client(
        &platform,
        Method::GET,
        "/servers",
        Some("alice-token"),
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let servers = response.json.as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["id"], alice.as_str());
    assert_eq!(servers[0]["suspended"], false);
    assert_eq!(servers[0]["allocations"][0]["name"], "game");
    // Vue client : ni nœud, ni réglages, ni propriétaire.
    for hidden in ["node", "settings", "secrets", "owner_ref", "state_reason"] {
        assert!(servers[0].get(hidden).is_none(), "{hidden}");
    }
}

#[tokio::test]
async fn another_owners_server_looks_missing_and_the_attempt_is_audited() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let bob = server_of(&platform, &mut daemon, json!(BOB)).await;

    let read = client(
        &platform,
        Method::GET,
        &format!("/servers/{bob}"),
        Some("alice-token"),
        None,
    )
    .await;
    let unknown = client(
        &platform,
        Method::GET,
        &format!("/servers/{}", Uuid::now_v7()),
        Some("alice-token"),
        None,
    )
    .await;

    assert_eq!(read.status, StatusCode::NOT_FOUND);
    assert_eq!(read.json["code"], unknown.json["code"]);
    let denied = platform.admin.last_audit("access.denied").await;
    assert_eq!(denied.outcome, "denied");
    assert_eq!(denied.actor_kind, "owner");
    assert_eq!(denied.actor_id, None);
    assert_eq!(denied.target_id.as_deref(), Some(bob.as_str()));
    assert_eq!(
        denied.details,
        json!({ "reason": "not_owner", "owner_ref": ALICE })
    );
}

#[tokio::test]
async fn an_owner_cannot_power_another_owners_server() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let bob = server_of(&platform, &mut daemon, json!(BOB)).await;

    let response = client(
        &platform,
        Method::POST,
        &format!("/servers/{bob}/power"),
        Some("alice-token"),
        Some(json!({ "action": "start" })),
    )
    .await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_owner_powers_their_server_and_is_the_audited_actor() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;

    let power = format!("/servers/{alice}/power");
    let (response, command) = tokio::join!(
        client(
            &platform,
            Method::POST,
            &power,
            Some("alice-token"),
            Some(json!({ "action": "start" })),
        ),
        daemon.accept(),
    );

    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.json);
    assert_eq!(command["type"], "server.power");
    assert_eq!(
        command["payload"],
        json!({ "server_id": alice, "action": "start" })
    );
    let event = platform.admin.last_audit("server.power").await;
    assert_eq!(event.actor_kind, "owner");
    assert_eq!(event.details["owner_ref"], ALICE);
    assert_eq!(event.details["action"], "start");
}

#[tokio::test]
async fn a_suspended_server_refuses_every_power_action_of_its_owner() {
    let platform = Platform::start_with_extensions(vec![shop()]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = server_of(&platform, &mut daemon, json!(ALICE)).await;
    let suspend = format!("/servers/{alice}/suspend");
    let (suspended, _) =
        tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept(),);
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.json);

    for action in ["start", "stop", "restart", "kill"] {
        let response = client(
            &platform,
            Method::POST,
            &format!("/servers/{alice}/power"),
            Some("alice-token"),
            Some(json!({ "action": action })),
        )
        .await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{action}");
        assert_eq!(response.json["code"], "server_suspended");
    }
    let read = client(
        &platform,
        Method::GET,
        &format!("/servers/{alice}"),
        Some("alice-token"),
        None,
    )
    .await;
    assert_eq!(read.json["suspended"], true);
}

#[tokio::test]
async fn two_extensions_cannot_both_provide_the_identity() {
    let pool = db::fresh_pool().await;
    let other = ExtensionFactory::new("other", |_| {
        Ok(Extension::default().identity(Arc::new(Tokens(HashMap::new()))))
    });

    let result = extension::prepare(&pool, &BTreeMap::new(), vec![shop(), other]).await;

    assert!(matches!(
        result,
        Err(ExtensionError::IdentityTwice { first, second }) if first == "shop" && second == "other"
    ));
}
