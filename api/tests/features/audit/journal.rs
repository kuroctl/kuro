use axum::http::{Method, StatusCode};
use kuro_api::audit::application;
use kuro_api::audit::domain::AuditContext;
use kuro_api::iam::domain::Permission;
use serde_json::json;

use crate::support::admin::{PASSWORD, TestAdmin};

/// Un peu d'activité : comptes, connexions, échec, rôle.
async fn activity(admin: &TestAdmin) {
    let root = admin.super_admin("root@example.com").await;
    let session = admin.login(&root).await;
    admin.login_with(&root.email, "wrong password").await;
    admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(json!({ "name": "support", "permissions": ["audit.read"] })),
        )
        .await;
}

#[tokio::test]
async fn the_log_is_append_only() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;

    for statement in [
        "UPDATE audit_log SET action = 'forged'",
        "DELETE FROM audit_log",
        "TRUNCATE audit_log",
    ] {
        let error = sqlx::query(statement)
            .execute(&admin.pool)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("append-only"),
            "{statement}: {error}"
        );
    }
}

#[tokio::test]
async fn the_hash_chain_is_intact_after_activity() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;

    let report = application::verify(&admin.pool).await.unwrap();

    assert!(report.is_intact());
    assert_eq!(
        usize::try_from(report.checked).unwrap(),
        admin.audit().await.len()
    );
}

#[tokio::test]
async fn tampering_is_detected() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;
    let target: i64 = sqlx::query_scalar("SELECT seq FROM audit_log ORDER BY seq LIMIT 1 OFFSET 2")
        .fetch_one(&admin.pool)
        .await
        .unwrap();

    // Contourner le trigger exige d'être propriétaire de la table : le chaînage le détecte quand même.
    sqlx::raw_sql("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_update_delete")
        .execute(&admin.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET outcome = 'success' WHERE seq = $1")
        .bind(target)
        .execute(&admin.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE audit_log SET details = '{\"forged\": true}' WHERE seq = $1")
        .bind(target)
        .execute(&admin.pool)
        .await
        .unwrap();

    let report = application::verify(&admin.pool).await.unwrap();

    assert_eq!(report.broken_at, Some(target));
}

#[tokio::test]
async fn deleting_a_middle_event_breaks_the_chain() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;
    let target: i64 = sqlx::query_scalar("SELECT seq FROM audit_log ORDER BY seq LIMIT 1 OFFSET 2")
        .fetch_one(&admin.pool)
        .await
        .unwrap();
    sqlx::raw_sql("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_update_delete")
        .execute(&admin.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM audit_log WHERE seq = $1")
        .bind(target)
        .execute(&admin.pool)
        .await
        .unwrap();

    let report = application::verify(&admin.pool).await.unwrap();

    assert!(report.broken_at.is_some_and(|seq| seq > target));
}

#[tokio::test]
async fn retention_purge_removes_old_events_and_is_itself_audited() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;
    let before = admin.audit().await.len();

    let deleted = application::purge(&admin.pool, &AuditContext::cli("test"), 0)
        .await
        .unwrap();

    assert_eq!(usize::try_from(deleted).unwrap(), before);
    let remaining = admin.audit().await;
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].action, "audit.purged");
    assert_eq!(remaining[0].actor_kind, "cli");
    assert!(application::verify(&admin.pool).await.unwrap().is_intact());
}

#[tokio::test]
async fn reading_the_log_is_itself_audited_and_paginated() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;
    let auditor = admin
        .account("auditor@example.com", &[Permission::AuditRead])
        .await;
    let session = admin.login(&auditor).await;

    let first = admin
        .call(
            Method::GET,
            "/audit?limit=3",
            Some(&session.access_token),
            None,
        )
        .await;

    assert_eq!(first.status, StatusCode::OK);
    let items = first.json["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let next = first.json["next_before_seq"].as_i64().unwrap();
    let second = admin
        .call(
            Method::GET,
            &format!("/audit?limit=3&before_seq={next}"),
            Some(&session.access_token),
            None,
        )
        .await;
    assert!(second.json["items"][0]["seq"].as_i64().unwrap() < next);
    let reads = admin
        .audit()
        .await
        .into_iter()
        .filter(|row| row.action == "audit.read" && row.actor_id == Some(auditor.id))
        .count();
    assert_eq!(reads, 2);
}

#[tokio::test]
async fn failed_mutations_leave_no_trace() {
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

    let duplicate = admin
        .call(
            Method::POST,
            "/roles",
            Some(&session.access_token),
            Some(body),
        )
        .await;

    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    let created = admin
        .audit()
        .await
        .into_iter()
        .filter(|row| row.action == "iam.role.created")
        .count();
    assert_eq!(created, 1);
}

#[tokio::test]
async fn secrets_never_reach_the_log() {
    let admin = TestAdmin::start().await;
    activity(&admin).await;
    let session = admin
        .login(&admin.account("alice@example.com", &[]).await)
        .await;
    admin
        .call_with_cookie(Method::POST, "/auth/refresh", &session.refresh_cookie)
        .await;

    let dump: Vec<String> = sqlx::query_scalar(
        "SELECT concat_ws(' ', action, target_id, user_agent, details::text) FROM audit_log",
    )
    .fetch_all(&admin.pool)
    .await
    .unwrap();
    let dump = dump.join("\n");

    for secret in [
        PASSWORD,
        "wrong password",
        &session.access_token,
        &session.refresh_cookie,
    ] {
        assert!(!dump.contains(secret), "secret found in audit log");
    }
    assert!(!dump.contains("@example.com"), "email found in audit log");
}
