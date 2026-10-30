//! Régions : emplacement physique des nœuds.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::admin::{Session, TestAdmin};

async fn root(admin: &TestAdmin) -> Session {
    admin
        .login(&admin.super_admin("root@example.com").await)
        .await
}

async fn create_region(admin: &TestAdmin, session: &Session, slug: &str) -> Value {
    let response = admin
        .call(
            Method::POST,
            "/regions",
            Some(&session.access_token),
            Some(json!({ "slug": slug, "name": "Europe Ouest", "latitude": 48.85, "longitude": 2.35 })),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json
}

#[tokio::test]
async fn region_lifecycle_is_audited() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let token = Some(session.access_token.as_str());
    let region = create_region(&admin, &session, "eu-west").await;
    let id = region["id"].as_str().unwrap();
    assert_eq!(region["nodes"]["total"], 0);

    let updated = admin
        .call(
            Method::PATCH,
            &format!("/regions/{id}"),
            token,
            Some(json!({ "name": "Europe de l'Ouest", "latitude": 50.0, "longitude": 4.0 })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["slug"], "eu-west");
    assert_eq!(updated.json["latitude"], 50.0);

    let deleted = admin
        .call(Method::DELETE, &format!("/regions/{id}"), token, None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    for action in ["region.created", "region.updated", "region.deleted"] {
        assert_eq!(
            admin.last_audit(action).await.target_id.as_deref(),
            Some(id)
        );
    }
}

#[tokio::test]
async fn region_input_is_validated() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    create_region(&admin, &session, "eu-west").await;

    for (body, status, error) in [
        (
            json!({ "slug": "eu-west", "name": "Dup", "latitude": 0.0, "longitude": 0.0 }),
            StatusCode::CONFLICT,
            "region_slug_taken",
        ),
        (
            json!({ "slug": "EU West", "name": "X", "latitude": 0.0, "longitude": 0.0 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_region_slug",
        ),
        (
            json!({ "slug": "x", "name": "X", "latitude": 91.0, "longitude": 0.0 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_coordinates",
        ),
        (
            json!({ "slug": "x", "name": "", "latitude": 0.0, "longitude": 0.0 }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_region_name",
        ),
    ] {
        let response = admin
            .call(
                Method::POST,
                "/regions",
                Some(&session.access_token),
                Some(body),
            )
            .await;
        assert_eq!(response.status, status, "{}", response.json);
        assert_eq!(response.json["code"], error);
    }
}

#[tokio::test]
async fn a_region_with_nodes_cannot_be_deleted() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let token = Some(session.access_token.as_str());
    let region = create_region(&admin, &session, "eu-west").await;
    let id = region["id"].as_str().unwrap();
    admin
        .call(
            Method::POST,
            "/nodes",
            token,
            Some(json!({ "name": "par-01", "region_id": id })),
        )
        .await;

    let deleted = admin
        .call(Method::DELETE, &format!("/regions/{id}"), token, None)
        .await;
    let listed = admin.call(Method::GET, "/regions", token, None).await;

    assert_eq!(deleted.status, StatusCode::CONFLICT);
    assert_eq!(deleted.json["code"], "region_not_empty");
    assert_eq!(listed.json[0]["nodes"]["total"], 1);
    assert_eq!(listed.json[0]["nodes"]["pending"], 1);
}
