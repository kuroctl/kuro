//! Rôles dynamiques : catalogue, validation des permissions, anti-escalade, rôle système.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::{Permission, SUPER_ADMIN_ROLE_ID};
use serde_json::json;

use crate::support::admin::TestAdmin;

#[tokio::test]
async fn catalog_lists_permissions_by_category() {
    let admin = TestAdmin::start().await;
    let account = admin
        .account("alice@example.com", &[Permission::IamRolesRead])
        .await;
    let session = admin.login(&account).await;

    let catalog = admin
        .call(
            Method::GET,
            "/permissions",
            Some(&session.access_token),
            None,
        )
        .await;

    assert_eq!(catalog.status, StatusCode::OK);
    let categories = catalog.json.as_array().unwrap();
    // Hors catégorie `extensions`, que remplissent d'autres tests du binaire (ADR-0051).
    let count: usize = categories
        .iter()
        .filter(|category| category["key"] != "extensions")
        .map(|category| category["permissions"].as_array().unwrap().len())
        .sum();
    assert_eq!(count, Permission::ALL.len());
    let iam = categories
        .iter()
        .find(|category| category["key"] == "iam")
        .unwrap();
    let credentials = iam["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|permission| permission["key"] == "iam.users.credentials")
        .unwrap();
    assert_eq!(credentials["sensitive"], true);
    assert_eq!(credentials["availability"], "available");
}

#[tokio::test]
async fn role_lifecycle_is_audited() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let token = Some(session.access_token.as_str());

    let created = admin
        .call(
            Method::POST,
            "/roles",
            token,
            Some(json!({ "name": "support-n1", "description": "Support", "permissions": ["iam.users.read"] })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    let id = created.json["id"].as_str().unwrap().to_owned();

    let updated = admin
        .call(
            Method::PATCH,
            &format!("/roles/{id}"),
            token,
            Some(json!({ "permissions": ["iam.users.read", "audit.read"] })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(
        updated.json["permissions"],
        json!(["audit.read", "iam.users.read"])
    );

    let deleted = admin
        .call(Method::DELETE, &format!("/roles/{id}"), token, None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let created_event = admin.last_audit("iam.role.created").await;
    assert_eq!(created_event.actor_id, Some(root.id));
    assert_eq!(created_event.target_id.as_deref(), Some(id.as_str()));
    let updated_event = admin.last_audit("iam.role.updated").await;
    assert_eq!(updated_event.details["added"], json!(["audit.read"]));
    admin.last_audit("iam.role.deleted").await;
}

#[tokio::test]
async fn invalid_permissions_are_refused() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;

    for (permission, error) in [
        ("nodes.teleport", "unknown_permission"),
        ("servers.transfer", "permission_not_assignable"),
        ("*", "permission_not_assignable"),
    ] {
        let response = admin
            .call(
                Method::POST,
                "/roles",
                Some(&session.access_token),
                Some(json!({ "name": "test", "permissions": [permission] })),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{permission}"
        );
        assert_eq!(response.json["code"], error, "{permission}");
        assert_eq!(response.json["params"]["permission"], permission);
    }
}

#[tokio::test]
async fn a_role_cannot_grant_permissions_its_author_lacks() {
    let admin = TestAdmin::start().await;
    let manager = admin
        .account(
            "manager@example.com",
            &[Permission::IamRolesManage, Permission::IamUsersRead],
        )
        .await;
    let session = admin.login(&manager).await;

    let escalation = admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(json!({ "name": "evil", "permissions": ["iam.users.read", "iam.users.delete"] })),
        )
        .await;
    let allowed = admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(json!({ "name": "readers", "permissions": ["iam.users.read"] })),
        )
        .await;

    assert_eq!(escalation.status, StatusCode::FORBIDDEN);
    assert_eq!(escalation.json["code"], "cannot_grant_permissions_you_lack");
    assert_eq!(allowed.status, StatusCode::CREATED);
}

#[tokio::test]
async fn system_role_is_untouchable() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let path = format!("/roles/{SUPER_ADMIN_ROLE_ID}");

    let update = admin
        .call(
            Method::PATCH,
            &path,
            Some(&session.access_token),
            Some(json!({ "description": "x" })),
        )
        .await;
    let delete = admin
        .call(Method::DELETE, &path, Some(&session.access_token), None)
        .await;

    assert_eq!(update.status, StatusCode::FORBIDDEN);
    assert_eq!(update.json["code"], "system_role");
    assert_eq!(delete.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn duplicate_role_name_is_a_conflict() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let body = json!({ "name": "support", "permissions": [] });
    admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(body.clone()),
        )
        .await;

    let again = admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(body),
        )
        .await;

    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json["code"], "role_name_taken");
}

#[tokio::test]
async fn roles_carry_a_badge_color() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    let token = Some(session.access_token.as_str());

    let default = admin
        .call(
            Method::POST,
            "/roles",
            token,
            Some(json!({ "name": "plain", "permissions": [] })),
        )
        .await;
    let colored = admin
        .call(
            Method::POST,
            "/roles",
            token,
            Some(json!({ "name": "ops", "color": "#6366F1", "permissions": [] })),
        )
        .await;
    let invalid = admin
        .call(
            Method::POST,
            "/roles",
            token,
            Some(json!({ "name": "bad", "color": "indigo", "permissions": [] })),
        )
        .await;
    let recolored = admin
        .call(
            Method::PATCH,
            &format!("/roles/{}", colored.json["id"].as_str().unwrap()),
            token,
            Some(json!({ "color": "#10b981" })),
        )
        .await;

    assert_eq!(default.json["color"], "#64748b");
    assert_eq!(colored.json["color"], "#6366f1");
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.json["code"], "invalid_color");
    assert_eq!(recolored.json["color"], "#10b981");
    assert_eq!(
        admin.last_audit("iam.role.updated").await.details["color"],
        "#10b981"
    );
}
