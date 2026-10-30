//! Consultation du journal depuis le panel (ADR-0019) : filtres, vérification de la chaîne,
//! export CSV ; chacune est elle-même journalisée.

use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{Method, StatusCode};
use kuro_api::audit::application;
use kuro_api::audit::domain::{AuditAction, AuditContext, AuditEvent};
use kuro_api::iam::domain::Permission;

use crate::support::admin::TestAdmin;

/// Auditeur connecté, après un échec de connexion (issue `failure`) et une connexion réussie.
async fn auditor(admin: &TestAdmin) -> String {
    let auditor = admin
        .account("auditor@example.com", &[Permission::AuditRead])
        .await;
    admin.login_with(&auditor.email, "wrong password").await;
    admin.login(&auditor).await.access_token
}

#[tokio::test]
async fn the_list_filters_by_actor_kind_outcome_and_period() {
    let admin = TestAdmin::start().await;
    let token = auditor(&admin).await;
    application::record(
        &admin.pool,
        &AuditContext::system(),
        AuditEvent::success(AuditAction::AuditPurged),
    )
    .await
    .unwrap();

    let system = admin
        .call(Method::GET, "/audit?actor_kind=system", Some(&token), None)
        .await;
    let failed = admin
        .call(Method::GET, "/audit?outcome=failure", Some(&token), None)
        .await;
    let future = admin
        .call(
            Method::GET,
            "/audit?from=2999-01-01T00:00:00Z",
            Some(&token),
            None,
        )
        .await;
    let past = admin
        .call(
            Method::GET,
            "/audit?to=2000-01-01T00:00:00Z",
            Some(&token),
            None,
        )
        .await;

    assert_eq!(system.status, StatusCode::OK, "{}", system.json);
    let items = system.json["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().all(|i| i["actor_kind"] == "system"));
    let items = failed.json["items"].as_array().unwrap();
    assert!(!items.is_empty());
    assert!(items.iter().all(|i| i["outcome"] == "failure"));
    assert_eq!(future.json["items"], serde_json::json!([]));
    assert_eq!(past.json["items"], serde_json::json!([]));
}

#[tokio::test]
async fn the_chain_is_verified_on_demand_and_the_check_is_audited() {
    let admin = TestAdmin::start().await;
    let token = auditor(&admin).await;

    let response = admin
        .call(Method::POST, "/audit/verify", Some(&token), None)
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    assert_eq!(response.json["intact"], true);
    assert!(response.json["checked"].as_u64().unwrap() > 0);
    assert!(response.json["broken_at"].is_null());
    let event = admin.last_audit("audit.verified").await;
    assert_eq!(event.details["broken_at"], serde_json::Value::Null);
}

#[tokio::test]
async fn a_broken_chain_is_reported_with_its_first_bad_event() {
    let admin = TestAdmin::start().await;
    let token = auditor(&admin).await;
    let rows = admin.audit().await;
    assert!(rows.len() > 2);
    // Altération directe, triggers désactivés le temps de la requête.
    let mut tx = admin.pool.begin().await.unwrap();
    sqlx::query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_update_delete")
        .execute(&mut *tx)
        .await
        .unwrap();
    let seq: i64 = sqlx::query_scalar(
        "UPDATE audit_log SET action = 'forged' WHERE seq = (SELECT min(seq) + 1 FROM audit_log)
         RETURNING seq",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("ALTER TABLE audit_log ENABLE TRIGGER audit_log_no_update_delete")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let response = admin
        .call(Method::POST, "/audit/verify", Some(&token), None)
        .await;

    assert_eq!(response.json["intact"], false);
    assert_eq!(response.json["broken_at"], seq);
}

#[tokio::test]
async fn the_export_is_a_filtered_csv_in_chain_order_and_is_audited() {
    let admin = TestAdmin::start().await;
    let token = auditor(&admin).await;
    // Un User-Agent qui ressemble à une formule de tableur.
    let context = AuditContext {
        user_agent: Some("=HYPERLINK(\"http://evil\")".to_owned()),
        ..AuditContext::system()
    };
    application::record(
        &admin.pool,
        &context,
        AuditEvent::success(AuditAction::AuditPurged),
    )
    .await
    .unwrap();

    let all = admin
        .call(Method::GET, "/audit/export", Some(&token), None)
        .await;
    let system = admin
        .call(
            Method::GET,
            "/audit/export?actor_kind=system",
            Some(&token),
            None,
        )
        .await;

    assert_eq!(all.status, StatusCode::OK);
    assert_eq!(
        all.headers[CONTENT_TYPE].to_str().unwrap(),
        "text/csv; charset=utf-8"
    );
    assert!(
        all.headers[CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment; filename=\"kuro-audit-")
    );
    let lines: Vec<&str> = all.body.lines().collect();
    assert!(lines[0].starts_with("seq,occurred_at,actor_kind,actor_id,action,outcome,"));
    let seqs: Vec<i64> = lines[1..]
        .iter()
        .map(|line| line.split(',').next().unwrap().parse().unwrap())
        .collect();
    assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "{seqs:?}");
    let system_lines: Vec<&str> = system.body.lines().skip(1).collect();
    assert_eq!(system_lines.len(), 1);
    assert!(
        system_lines[0].contains(",\"'=HYPERLINK(\"\"http://evil\"\")\","),
        "{}",
        system_lines[0]
    );
    let event = admin.last_audit("audit.exported").await;
    assert_eq!(event.details["actor_kind"], "system");
}

#[tokio::test]
async fn reviewing_needs_the_audit_permission() {
    let admin = TestAdmin::start().await;
    let other = admin
        .account("other@example.com", &[Permission::ServersRead])
        .await;
    let token = admin.login(&other).await.access_token;

    let verify = admin
        .call(Method::POST, "/audit/verify", Some(&token), None)
        .await;
    let export = admin
        .call(Method::GET, "/audit/export", Some(&token), None)
        .await;

    assert_eq!(verify.status, StatusCode::FORBIDDEN);
    assert_eq!(export.status, StatusCode::FORBIDDEN);
}
