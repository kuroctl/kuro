//! Loaders de la source de contenu et leurs icônes.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::json;

use crate::support::platform::Platform;

#[tokio::test]
async fn loaders_are_listed_with_their_icon() {
    let platform = Platform::start().await;

    let response = platform.call(Method::GET, "/content/loaders", None).await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json,
        json!([
            { "name": "fabric", "icon_svg": "<svg viewBox=\"0 0 24 24\"/>" },
            { "name": "paper", "icon_svg": null },
        ])
    );
}

#[tokio::test]
async fn listing_loaders_requires_reading_content() {
    let platform = Platform::start().await;
    let account = platform
        .admin
        .account("alice@example.com", &[Permission::ServersRead])
        .await;
    let session = platform.admin.login(&account).await;

    let response = platform
        .admin
        .call(
            Method::GET,
            "/content/loaders",
            Some(&session.access_token),
            None,
        )
        .await;

    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(
        response.json["params"]["permission"],
        "servers.content.read"
    );
}
