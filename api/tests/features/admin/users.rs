//! Comptes admin : cycle de vie, identifiants, anti-escalade, dernier super-admin.

use axum::http::{Method, StatusCode};
use kuro_api::audit::domain::AuditContext;
use kuro_api::error_code::ErrorCode;
use kuro_api::iam::application::{IamError, UserPatch, users};
use kuro_api::iam::domain::{Authority, Permission, SUPER_ADMIN_ROLE_ID};
use serde_json::json;

use crate::support::admin::{PASSWORD, REFRESH_COOKIE, TestAdmin};

#[tokio::test]
async fn created_account_is_listed_and_audited() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;

    let created = admin
        .call(
            Method::POST,
            "/users",
            Some(&session.access_token),
            Some(json!({ "email": "Bob@Example.com", "display_name": "Bob", "password": "a long enough password" })),
        )
        .await;

    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
    assert_eq!(created.json["email"], "bob@example.com");
    assert_eq!(created.json["roles"], json!([]));
    let list = admin
        .call(Method::GET, "/users", Some(&session.access_token), None)
        .await;
    assert_eq!(list.json.as_array().unwrap().len(), 2);
    let event = admin.last_audit("iam.user.created").await;
    assert_eq!(event.actor_id, Some(root.id));
    assert_eq!(event.target_id.as_deref(), created.json["id"].as_str());
}

#[tokio::test]
async fn account_input_is_validated() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;

    for (body, status, error) in [
        (
            json!({ "email": "root@example.com", "display_name": "Dup", "password": "a long enough password" }),
            StatusCode::CONFLICT,
            "email_taken",
        ),
        (
            json!({ "email": "weak@example.com", "display_name": "Weak", "password": "short" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "weak_password",
        ),
        (
            json!({ "email": "not-an-email", "display_name": "X", "password": "a long enough password" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_email",
        ),
    ] {
        let response = admin
            .call(
                Method::POST,
                "/users",
                Some(&session.access_token),
                Some(body),
            )
            .await;
        assert_eq!(response.status, status, "{}", response.json);
        assert_eq!(response.json["code"], error);
    }
}

#[tokio::test]
async fn nobody_acts_on_their_own_account_through_admin_routes() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    admin.super_admin("root2@example.com").await;
    let session = admin.login(&root).await;
    let token = Some(session.access_token.as_str());
    let me = format!("/users/{}", root.id);

    let disable = admin
        .call(Method::PATCH, &me, token, Some(json!({ "disabled": true })))
        .await;
    let delete = admin.call(Method::DELETE, &me, token, None).await;
    let roles = admin
        .call(
            Method::PUT,
            &format!("{me}/roles"),
            token,
            Some(json!({ "role_ids": [] })),
        )
        .await;

    assert_eq!(disable.json["code"], "cannot_disable_self");
    assert_eq!(delete.json["code"], "cannot_delete_self");
    assert_eq!(roles.json["code"], "cannot_change_own_roles");
}

/// Par le REST, seul un autre super-admin actif peut agir sur un super-admin : il en reste donc
/// toujours un. La règle protège les actions depuis l'hôte (CLI) et les modifications concurrentes.
#[tokio::test]
async fn the_last_active_super_admin_cannot_be_disabled_demoted_or_deleted() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let context = AuditContext::cli("test");
    let disable = UserPatch {
        disabled: Some(true),
        ..UserPatch::default()
    };

    let errors = [
        users::update(&admin.iam, &Authority::Host, &context, root.id, disable)
            .await
            .unwrap_err(),
        users::set_roles(&admin.iam, &Authority::Host, &context, root.id, &[])
            .await
            .unwrap_err(),
        users::delete(&admin.iam, &Authority::Host, &context, root.id)
            .await
            .unwrap_err(),
    ];

    for error in errors {
        assert!(
            matches!(error, IamError::Conflict(ErrorCode::LastSuperAdmin)),
            "{error}"
        );
    }
    admin.super_admin("root2@example.com").await;
    let disable = UserPatch {
        disabled: Some(true),
        ..UserPatch::default()
    };
    users::update(&admin.iam, &Authority::Host, &context, root.id, disable)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_admin_cannot_manage_someone_more_privileged() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let helpdesk = admin
        .account(
            "helpdesk@example.com",
            &[Permission::IamUsersCredentials, Permission::IamRolesAssign],
        )
        .await;
    let session = admin.login(&helpdesk).await;
    let token = Some(session.access_token.as_str());

    let reset = admin
        .call(
            Method::PUT,
            &format!("/users/{}/password", root.id),
            token,
            Some(json!({ "password": "attacker controlled pw" })),
        )
        .await;
    let demote_root = admin
        .call(
            Method::PUT,
            &format!("/users/{}/roles", root.id),
            token,
            Some(json!({ "role_ids": [] })),
        )
        .await;

    assert_eq!(reset.status, StatusCode::FORBIDDEN);
    assert_eq!(reset.json["code"], "insufficient_permissions_for_target");
    assert_eq!(demote_root.status, StatusCode::FORBIDDEN);
    assert_eq!(
        admin.login_with(&root.email, PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn super_admin_role_can_only_be_granted_by_a_super_admin() {
    let admin = TestAdmin::start().await;
    let assigner = admin
        .account("assigner@example.com", &[Permission::IamRolesAssign])
        .await;
    let target = admin.account("target@example.com", &[]).await;
    let session = admin.login(&assigner).await;

    let grant = admin
        .call(
            Method::PUT,
            &format!("/users/{}/roles", target.id),
            Some(&session.access_token),
            Some(json!({ "role_ids": [SUPER_ADMIN_ROLE_ID] })),
        )
        .await;

    assert_eq!(grant.status, StatusCode::FORBIDDEN);
    assert_eq!(grant.json["code"], "cannot_grant_permissions_you_lack");
}

#[tokio::test]
async fn password_reset_closes_the_target_sessions() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let root_session = admin.login(&root).await;
    let target = admin.account("target@example.com", &[]).await;
    let target_session = admin.login(&target).await;

    let reset = admin
        .call(
            Method::PUT,
            &format!("/users/{}/password", target.id),
            Some(&root_session.access_token),
            Some(json!({ "password": "a brand new password" })),
        )
        .await;

    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    let me = admin
        .call(Method::GET, "/me", Some(&target_session.access_token), None)
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        admin
            .login_with(&target.email, "a brand new password")
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        admin.last_audit("iam.user.password_reset").await.actor_id,
        Some(root.id)
    );
}

#[tokio::test]
async fn own_password_change_requires_the_current_one_and_keeps_this_session() {
    let admin = TestAdmin::start().await;
    let account = admin.account("alice@example.com", &[]).await;
    let other_session = admin.login(&account).await;
    let session = admin.login(&account).await;
    let token = Some(session.access_token.as_str());

    let wrong = admin
        .call(
            Method::PUT,
            "/me/password",
            token,
            Some(json!({ "current_password": "nope", "new_password": "a brand new password" })),
        )
        .await;
    let right = admin
        .call(
            Method::PUT,
            "/me/password",
            token,
            Some(json!({ "current_password": PASSWORD, "new_password": "a brand new password" })),
        )
        .await;

    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(right.status, StatusCode::NO_CONTENT);
    assert_eq!(
        admin.call(Method::GET, "/me", token, None).await.status,
        StatusCode::OK
    );
    let other = admin
        .call_with_cookie(Method::POST, "/auth/refresh", &other_session.refresh_cookie)
        .await;
    assert_eq!(other.status, StatusCode::UNAUTHORIZED);
    assert!(
        other
            .cookie(REFRESH_COOKIE)
            .is_some_and(|value| value.is_empty())
    );
    admin.last_audit("iam.self.password_changed").await;
}
