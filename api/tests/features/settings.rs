//! Paramètres du projet (ADR-0019) : lecture, modification auditée champ par champ, bornes,
//! permissions, conservation du journal d'audit.

use axum::http::{Method, StatusCode};
use kuro_api::audit::application;
use kuro_api::audit::domain::AuditContext;
use kuro_api::iam::domain::Permission;
use serde_json::json;

use crate::support::admin::TestAdmin;

async fn root(admin: &TestAdmin) -> String {
    let root = admin.super_admin("root@example.com").await;
    admin.login(&root).await.access_token
}

#[tokio::test]
async fn settings_start_from_the_previous_defaults() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;

    let response = admin
        .call(Method::GET, "/settings", Some(&token), None)
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let settings = &response.json;
    assert_eq!(settings["default_memory_mb"], 2048);
    assert_eq!(settings["default_cpu_percent"], 200);
    assert_eq!(settings["default_disk_mb"], 10240);
    assert_eq!(settings["default_backup_limit"], 3);
    assert_eq!(settings["audit_retention_days"], 365);
    for max in [
        "max_memory_mb",
        "max_cpu_percent",
        "max_disk_mb",
        "max_backup_limit",
    ] {
        assert!(settings[max].is_null(), "{max}");
    }
}

#[tokio::test]
async fn an_update_is_audited_with_the_old_and_new_value_of_each_field() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;

    let response = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({
                "default_memory_mb": 4096,
                "max_memory_mb": 8192,
                "default_cpu_percent": 200,
                "audit_retention_days": 90,
            })),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["max_memory_mb"], 8192);
    let event = admin.last_audit("system.settings_updated").await;
    assert_eq!(
        event.details["changed"],
        json!({
            "default_memory_mb": { "from": 2048, "to": 4096 },
            "max_memory_mb": { "from": null, "to": 8192 },
            "audit_retention_days": { "from": 365, "to": 90 },
        })
    );

    // `null` supprime un plafond.
    let cleared = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "max_memory_mb": null })),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.json);
    assert!(cleared.json["max_memory_mb"].is_null());
}

#[tokio::test]
async fn invalid_settings_are_refused_field_by_field() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;

    for (body, field, code) in [
        (
            json!({ "audit_retention_days": 7 }),
            "audit_retention_days",
            "value_out_of_range",
        ),
        (
            json!({ "default_memory_mb": 32 }),
            "default_memory_mb",
            "value_out_of_range",
        ),
        (
            json!({ "max_disk_mb": 0 }),
            "max_disk_mb",
            "value_out_of_range",
        ),
        (
            json!({ "default_memory_mb": 4096, "max_memory_mb": 1024 }),
            "default_memory_mb",
            "resource_above_ceiling",
        ),
        // Disque illimité par défaut, alors qu'un plafond existe.
        (
            json!({ "default_disk_mb": 0, "max_disk_mb": 20480 }),
            "default_disk_mb",
            "resource_above_ceiling",
        ),
    ] {
        let response = admin
            .call(Method::PATCH, "/settings", Some(&token), Some(body.clone()))
            .await;
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let fields = response.json["fields"].as_array().unwrap();
        assert!(
            fields
                .iter()
                .any(|f| f["field"] == field && f["code"] == code),
            "{body}: {}",
            response.json
        );
    }

    let unchanged = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "default_backup_limit": 3 })),
        )
        .await;
    assert_eq!(unchanged.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unchanged.json["code"], "empty_patch");
    assert!(
        admin
            .audit()
            .await
            .iter()
            .all(|row| row.action != "system.settings_updated")
    );
}

#[tokio::test]
async fn settings_need_their_permissions() {
    let admin = TestAdmin::start().await;
    let reader = admin
        .account("reader@example.com", &[Permission::SystemSettingsRead])
        .await;
    let reader = admin.login(&reader).await.access_token;
    let other = admin
        .account("other@example.com", &[Permission::ServersRead])
        .await;
    let other = admin.login(&other).await.access_token;

    let read = admin
        .call(Method::GET, "/settings", Some(&reader), None)
        .await;
    let update = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&reader),
            Some(json!({ "audit_retention_days": 90 })),
        )
        .await;
    let denied = admin
        .call(Method::GET, "/settings", Some(&other), None)
        .await;

    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(update.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(admin.last_audit("access.denied").await.outcome, "denied");
}

#[tokio::test]
async fn the_purge_applies_the_retention_of_the_settings() {
    let admin = TestAdmin::start().await;
    let token = root(&admin).await;
    admin
        .call(
            Method::PATCH,
            "/settings",
            Some(&token),
            Some(json!({ "audit_retention_days": 30 })),
        )
        .await;

    let (retention, deleted) = application::purge_expired(&admin.pool, &AuditContext::system())
        .await
        .unwrap();

    assert_eq!(retention, 30);
    // Tout est récent : rien n'est supprimé, mais la purge est tracée.
    assert_eq!(deleted, 0);
    let event = admin.last_audit("audit.purged").await;
    assert_eq!(event.actor_kind, "system");
    assert_eq!(event.details["retention_days"], 30);
}
