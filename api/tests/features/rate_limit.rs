//! Routes sans authentification : débit limité par adresse cliente, établie derrière les
//! proxys de confiance (`http.trusted_proxies`).

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::support::admin::TestAdmin;

/// Limite par défaut de `HttpConfig` : 30 requêtes par minute et par adresse.
const LIMIT: usize = 30;

struct Reply {
    status: StatusCode,
    retry_after: Option<String>,
    json: Value,
}

/// `POST /auth/refresh` sans jeton, depuis un pair donné, avec ou sans `X-Forwarded-For`.
async fn refresh_from(admin: &TestAdmin, peer: &str, forwarded: Option<&str>) -> Reply {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/admin/v1/auth/refresh");
    if let Some(forwarded) = forwarded {
        request = request.header("x-forwarded-for", forwarded);
    }
    let mut request = request.body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let response = admin.router().oneshot(request).await.unwrap();
    let status = response.status();
    let retry_after = response
        .headers()
        .get("retry-after")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Reply {
        status,
        retry_after,
        json,
    }
}

#[tokio::test]
async fn an_address_that_floods_a_public_route_is_told_to_wait() {
    let admin = TestAdmin::start().await;

    for _ in 0..LIMIT {
        let reply = refresh_from(&admin, "198.51.100.7:40000", None).await;
        assert_ne!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    }
    let limited = refresh_from(&admin, "198.51.100.7:40001", None).await;

    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.json["code"], "rate_limited");
    assert!(limited.json["params"]["retry_after_secs"].as_u64().unwrap() >= 1);
    assert!(limited.retry_after.is_some());
    // Une autre adresse n'est pas concernée.
    let other = refresh_from(&admin, "198.51.100.8:40000", None).await;
    assert_ne!(other.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_direct_peer_cannot_escape_the_limit_by_announcing_other_addresses() {
    let admin = TestAdmin::start().await;

    for _ in 0..LIMIT {
        refresh_from(&admin, "198.51.100.7:40000", Some("203.0.113.1")).await;
    }
    let spoofed = refresh_from(&admin, "198.51.100.7:40000", Some("203.0.113.2")).await;

    assert_eq!(spoofed.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn behind_the_local_proxy_the_forwarded_address_is_the_client() {
    let admin = TestAdmin::start().await;

    for _ in 0..LIMIT {
        refresh_from(&admin, "127.0.0.1:40000", Some("203.0.113.5")).await;
    }
    let same_client = refresh_from(&admin, "127.0.0.1:40000", Some("203.0.113.5")).await;
    let other_client = refresh_from(&admin, "127.0.0.1:40000", Some("203.0.113.6")).await;

    assert_eq!(same_client.status, StatusCode::TOO_MANY_REQUESTS);
    assert_ne!(other_client.status, StatusCode::TOO_MANY_REQUESTS);
}
