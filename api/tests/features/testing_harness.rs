//! Harnais public des extensions (`kuro_api::testing`, ADR-0051) : utilisé comme le fera le
//! dépôt d'une extension, hors des helpers internes de Kuro.

use std::collections::BTreeMap;

use axum::Router;
use axum::http::{Method, StatusCode};
use axum::routing::get;
use kuro_api::extension::{Extension, ExtensionFactory};
use kuro_api::iam::domain::{Permission, PermissionDef};
use kuro_api::testing::TestKuro;
use serde_json::json;

use crate::support::db;

static PERMS: [PermissionDef; 1] = [PermissionDef {
    key: "harness.things.read",
    label: "Voir les choses",
    sensitive: false,
    agent_grantable: false,
}];

fn harness() -> ExtensionFactory {
    ExtensionFactory::new("harness", |_| {
        Ok(Extension::default().routes(Router::new().route(
            "/v1/ping",
            get(|| async { axum::Json(json!({ "pong": true })) }),
        )))
    })
    .permissions(&PERMS)
}

#[tokio::test]
async fn an_extension_mounts_kuro_and_calls_it_as_an_admin() {
    // Base vierge : le harnais applique lui-même les migrations de Kuro.
    let pool = db::empty_pool().await;
    let kuro = TestKuro::start(pool, BTreeMap::new(), vec![harness()])
        .await
        .unwrap();

    let ping = kuro
        .request(Method::GET, "/api/harness/v1/ping", None, None)
        .await
        .unwrap();
    let reader = kuro
        .admin_token("reader@example.com", &[Permission::of(&PERMS[0])])
        .await
        .unwrap();
    let root = kuro.super_admin_token("root@example.com").await.unwrap();
    let me = kuro
        .request(Method::GET, "/api/admin/v1/me", Some(&reader), None)
        .await
        .unwrap();
    let servers = kuro
        .request(Method::GET, "/api/admin/v1/servers", Some(&root), None)
        .await
        .unwrap();

    assert_eq!(ping.json, json!({ "pong": true }));
    assert_eq!(me.status, StatusCode::OK, "{}", me.json);
    assert_eq!(me.json["permissions"], json!(["harness.things.read"]));
    assert_eq!(servers.status, StatusCode::OK);
    assert_eq!(servers.json, json!([]));
}
