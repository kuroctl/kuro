//! Nœuds par le REST : création (token montré une fois), lecture, emplacement, réémission.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::admin::{Session, TestAdmin};
use crate::support::platform::Platform;

async fn root(admin: &TestAdmin) -> Session {
    admin
        .login(&admin.super_admin("root@example.com").await)
        .await
}

async fn region(
    admin: &TestAdmin,
    session: &Session,
    slug: &str,
    latitude: f64,
    longitude: f64,
) -> String {
    let response = admin
        .call(
            Method::POST,
            "/regions",
            Some(&session.access_token),
            Some(
                json!({ "slug": slug, "name": slug, "latitude": latitude, "longitude": longitude }),
            ),
        )
        .await;
    response.json["id"].as_str().unwrap().to_owned()
}

async fn create_node(admin: &TestAdmin, session: &Session, body: Value) -> Value {
    let response = admin
        .call(
            Method::POST,
            "/nodes",
            Some(&session.access_token),
            Some(body),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json
}

#[tokio::test]
async fn created_node_comes_with_a_one_time_enrollment_ticket() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let region_id = region(&admin, &session, "eu-west", 48.85, 2.35).await;

    let created = create_node(
        &admin,
        &session,
        json!({ "name": "par-01", "display_name": "Paris 1", "region_id": region_id, "location": "Paris – OVH GRA" }),
    )
    .await;

    let node = &created["node"];
    assert_eq!(node["status"], "pending");
    assert_eq!(node["display_name"], "Paris 1");
    assert_eq!(node["region"]["slug"], "eu-west");
    assert_eq!(
        node["map_position"],
        json!({ "latitude": 48.85, "longitude": 2.35 })
    );
    assert!(node["host"].is_null());
    let token = created["enrollment"]["token"].as_str().unwrap();
    assert!(token.starts_with("kuro_enr_"));
    assert_eq!(
        created["enrollment"]["ca_fingerprint"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    let fetched = admin
        .call(
            Method::GET,
            &format!("/nodes/{}", node["id"].as_str().unwrap()),
            Some(&session.access_token),
            None,
        )
        .await;
    assert!(!fetched.json.to_string().contains(token));
    let event = admin.last_audit("node.added").await;
    assert_eq!(event.actor_kind, "admin");
    assert!(!event.details.to_string().contains(token));
}

#[tokio::test]
async fn node_creation_is_validated() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    create_node(&admin, &session, json!({ "name": "par-01" })).await;

    for (body, status, error) in [
        (
            json!({ "name": "par-01" }),
            StatusCode::CONFLICT,
            "node_name_taken",
        ),
        (
            json!({ "name": "Par 01" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_node_name",
        ),
        (
            json!({ "name": "x", "region_id": uuid::Uuid::now_v7() }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_region",
        ),
        (
            json!({ "name": "x", "coordinates": { "latitude": 0.0, "longitude": 200.0 } }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_coordinates",
        ),
    ] {
        let response = admin
            .call(
                Method::POST,
                "/nodes",
                Some(&session.access_token),
                Some(body),
            )
            .await;
        assert_eq!(response.status, status, "{}", response.json);
        assert_eq!(response.json["code"], error);
    }
}

#[tokio::test]
async fn nodes_can_be_listed_by_region() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let west = region(&admin, &session, "eu-west", 48.85, 2.35).await;
    let east = region(&admin, &session, "us-east", 39.0, -77.0).await;
    create_node(
        &admin,
        &session,
        json!({ "name": "par-01", "region_id": west }),
    )
    .await;
    create_node(
        &admin,
        &session,
        json!({ "name": "ash-01", "region_id": east }),
    )
    .await;
    create_node(&admin, &session, json!({ "name": "lab-01" })).await;
    let token = Some(session.access_token.as_str());

    let names = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["name"].as_str().unwrap().to_owned())
            .collect()
    };
    let all = admin.call(Method::GET, "/nodes", token, None).await;
    let in_west = admin
        .call(
            Method::GET,
            &format!("/nodes?region_id={west}"),
            token,
            None,
        )
        .await;
    let unassigned = admin
        .call(Method::GET, "/nodes?unassigned=true", token, None)
        .await;

    assert_eq!(names(&all.json), ["ash-01", "lab-01", "par-01"]);
    assert_eq!(names(&in_west.json), ["par-01"]);
    assert_eq!(names(&unassigned.json), ["lab-01"]);
}

#[tokio::test]
async fn placement_and_label_can_be_changed_and_cleared() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let west = region(&admin, &session, "eu-west", 48.85, 2.35).await;
    let created = create_node(
        &admin,
        &session,
        json!({ "name": "par-01", "display_name": "Paris" }),
    )
    .await;
    let path = format!("/nodes/{}", created["node"]["id"].as_str().unwrap());
    let token = Some(session.access_token.as_str());

    let moved = admin
        .call(
            Method::PATCH,
            &path,
            token,
            Some(json!({ "region_id": west, "coordinates": { "latitude": 45.76, "longitude": 4.83 } })),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.json);
    assert_eq!(moved.json["display_name"], "Paris");
    assert_eq!(
        moved.json["map_position"],
        json!({ "latitude": 45.76, "longitude": 4.83 })
    );

    let cleared = admin
        .call(
            Method::PATCH,
            &path,
            token,
            Some(json!({ "display_name": null, "coordinates": null })),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.json);
    assert!(cleared.json["display_name"].is_null());
    assert_eq!(
        cleared.json["map_position"],
        json!({ "latitude": 48.85, "longitude": 2.35 })
    );
    assert_eq!(cleared.json["name"], "par-01");

    let event = admin.last_audit("node.updated").await;
    assert_eq!(
        event.details["fields"],
        json!(["display_name", "coordinates"])
    );
}

#[tokio::test]
async fn node_update_is_validated() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let created = create_node(&admin, &session, json!({ "name": "par-01" })).await;
    let path = format!("/nodes/{}", created["node"]["id"].as_str().unwrap());
    let token = Some(session.access_token.as_str());

    for (body, error) in [
        (json!({}), "empty_patch"),
        (json!({ "display_name": "" }), "invalid_display_name"),
        (
            json!({ "region_id": uuid::Uuid::now_v7() }),
            "unknown_region",
        ),
        (
            json!({ "coordinates": { "latitude": -91.0, "longitude": 0.0 } }),
            "invalid_coordinates",
        ),
    ] {
        let response = admin.call(Method::PATCH, &path, token, Some(body)).await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            response.json
        );
        assert_eq!(response.json["code"], error);
    }
}

#[tokio::test]
async fn enrollment_token_can_be_reissued_for_a_pending_node_only() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let created = create_node(&admin, &session, json!({ "name": "par-01" })).await;
    let id = created["node"]["id"].as_str().unwrap();
    let path = format!("/nodes/{id}/enrollment-token");
    let token = Some(session.access_token.as_str());

    let reissued = admin.call(Method::POST, &path, token, None).await;
    assert_eq!(reissued.status, StatusCode::OK, "{}", reissued.json);
    assert_ne!(
        reissued.json["enrollment"]["token"],
        created["enrollment"]["token"]
    );
    assert_eq!(
        admin.last_audit("node.enrollment_reissued").await.details["invalidated_tokens"],
        1
    );

    sqlx::query("UPDATE nodes SET status = 'offline' WHERE id = $1::uuid")
        .bind(id)
        .execute(&admin.pool)
        .await
        .unwrap();
    let enrolled = admin.call(Method::POST, &path, token, None).await;
    assert_eq!(enrolled.status, StatusCode::CONFLICT);
    assert_eq!(enrolled.json["code"], "node_not_pending");

    let missing = admin
        .call(
            Method::POST,
            &format!("/nodes/{}/enrollment-token", uuid::Uuid::now_v7()),
            token,
            None,
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn revocation_rules_and_audit() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let created = create_node(&admin, &session, json!({ "name": "par-01" })).await;
    let id = created["node"]["id"].as_str().unwrap();
    let path = format!("/nodes/{id}/revoke");
    let token = Some(session.access_token.as_str());

    let pending = admin
        .call(Method::POST, &path, token, Some(json!({})))
        .await;
    assert_eq!(pending.status, StatusCode::CONFLICT);
    assert_eq!(pending.json["code"], "node_not_enrolled");

    sqlx::query("UPDATE nodes SET status = 'offline' WHERE id = $1::uuid")
        .bind(id)
        .execute(&admin.pool)
        .await
        .unwrap();
    let revoked = admin
        .call(
            Method::POST,
            &path,
            token,
            Some(json!({ "reason": "decommissioned" })),
        )
        .await;
    assert_eq!(revoked.status, StatusCode::OK, "{}", revoked.json);
    assert_eq!(revoked.json["status"], "revoked");
    let event = admin.last_audit("node.revoked").await;
    assert_eq!(event.details["reason"], "decommissioned");
    assert_eq!(event.details["previous_status"], "offline");

    let twice = admin
        .call(Method::POST, &path, token, Some(json!({})))
        .await;
    assert_eq!(twice.json["code"], "node_already_revoked");

    let reissued = admin
        .call(
            Method::POST,
            &format!("/nodes/{id}/enrollment-token"),
            token,
            None,
        )
        .await;
    assert_eq!(reissued.status, StatusCode::OK, "{}", reissued.json);
    assert_eq!(reissued.json["node"]["status"], "pending");
    assert_eq!(
        admin.last_audit("node.enrollment_reissued").await.details["reenrollment"],
        true
    );
}

#[tokio::test]
async fn a_pending_node_is_deleted_with_its_tokens_and_audited() {
    let admin = TestAdmin::start().await;
    let session = root(&admin).await;
    let created = create_node(&admin, &session, json!({ "name": "par-01" })).await;
    let id = created["node"]["id"].as_str().unwrap();
    let token = Some(session.access_token.as_str());

    let deleted = admin
        .call(Method::DELETE, &format!("/nodes/{id}"), token, None)
        .await;
    let again = admin
        .call(Method::DELETE, &format!("/nodes/{id}"), token, None)
        .await;

    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    let tokens: i64 = sqlx::query_scalar("SELECT count(*) FROM enrollment_tokens")
        .fetch_one(&admin.pool)
        .await
        .unwrap();
    assert_eq!(tokens, 0);
    let event = admin.last_audit("node.deleted").await;
    assert_eq!(event.details["name"], "par-01");
    assert_eq!(event.details["status"], "pending");
    assert_eq!(event.details["certificate_revoked"], false);
    assert_eq!(event.target_id.as_deref(), Some(id));
}

/// Un nœud en ligne, puis hors ligne avec un serveur, ne se supprime pas ; une fois le serveur
/// parti, la suppression révoque son certificat.
#[tokio::test]
async fn deletion_refuses_an_online_node_and_a_node_still_hosting_servers() {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let node = daemon.node.id;
    let path = format!("/nodes/{node}");
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;

    let online = platform.call(Method::DELETE, &path, None).await;
    assert_eq!(online.status, StatusCode::CONFLICT, "{}", online.json);
    assert_eq!(online.json["code"], "node_online");

    drop(daemon);
    platform.api.wait_for_status(node, "offline").await;
    let busy = platform.call(Method::DELETE, &path, None).await;
    assert_eq!(busy.status, StatusCode::CONFLICT, "{}", busy.json);
    assert_eq!(busy.json["code"], "node_has_servers");
    assert_eq!(platform.api.status(node).await, "offline");

    // Le serveur part (supprimé ou déplacé) : le nœud peut être supprimé.
    sqlx::query(
        "UPDATE allocations SET server_id = NULL, port_name = NULL WHERE server_id = $1::uuid",
    )
    .bind(server["id"].as_str().unwrap())
    .execute(&platform.api.pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM servers WHERE id = $1::uuid")
        .bind(server["id"].as_str().unwrap())
        .execute(&platform.api.pool)
        .await
        .unwrap();
    let deleted = platform.call(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json);

    let revoked: Vec<String> = sqlx::query_scalar("SELECT node_name FROM revoked_certificates")
        .fetch_all(&platform.api.pool)
        .await
        .unwrap();
    assert_eq!(revoked, vec!["node-1".to_owned()]);
    let event = platform.admin.last_audit("node.deleted").await;
    assert_eq!(event.details["status"], "offline");
    assert_eq!(event.details["certificate_revoked"], true);
}
