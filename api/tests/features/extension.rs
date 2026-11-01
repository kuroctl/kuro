//! Extensions compilées (ADR-0051) : routes sous `/api/<nom>`, tables dans le schéma `<nom>`,
//! section de configuration, noms vérifiés. Une extension de test, `demo`, ajoute une table de
//! notes et deux routes.

use std::collections::BTreeMap;

use axum::Json;
use axum::Router;
use axum::extract::Query;
use axum::extract::State;
use axum::http::{Method, StatusCode};
use axum::routing::{get, post};
use kuro_api::audit::domain::{AuditAction, AuditActionDef, AuditEvent};
use kuro_api::audit::infra::AuditedTx;
use kuro_api::error_code::ErrorCodeDef;
use kuro_api::extension::{self, Extension, ExtensionError, ExtensionFactory};
use kuro_api::http::error::{ApiError, ErrorCode, Problem, internal_error};
use kuro_api::iam::application::IamState;
use kuro_api::iam::domain::{Permission, PermissionDef};
use kuro_api::iam::http::AdminSession;
use kuro_api::state::AppState;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::migrate::Migrator;

use crate::support::admin::TestAdmin;
use crate::support::db;

static DEMO_MIGRATIONS: Migrator = sqlx::migrate!("tests/features/extension_migrations");

/// Permissions de l'extension de test, déclarées comme le fera une vraie extension.
static DEMO_PERMISSIONS: [PermissionDef; 1] = [PermissionDef {
    key: "demo.notes.purge",
    label: "Purger les notes",
    sensitive: true,
    agent_grantable: false,
}];
static NOTES_PURGE: &PermissionDef = &DEMO_PERMISSIONS[0];

/// Action d'audit et code d'erreur de l'extension de test.
static DEMO_AUDIT: [AuditActionDef; 1] = [AuditActionDef {
    code: "demo.notes.purged",
}];
static DEMO_ERRORS: [ErrorCodeDef; 1] = [ErrorCodeDef {
    code: "demo.notes.locked",
    title: "These notes are locked",
}];

fn demo() -> ExtensionFactory {
    ExtensionFactory::new("demo", |context| {
        let greeting = context
            .config
            .and_then(|config| config.get("greeting"))
            .and_then(Value::as_str)
            .unwrap_or("hello")
            .to_owned();
        Ok(Extension::default()
            .routes(
                Router::new()
                    .route("/v1/notes", get(count_notes))
                    .route(
                        "/v1/greeting",
                        get(move || async move { Json(json!({ "greeting": greeting })) }),
                    )
                    .route("/v1/admin/notes/purge", post(purge_notes)),
            )
            .migrations(&DEMO_MIGRATIONS))
    })
    .permissions(&DEMO_PERMISSIONS)
    .audit_actions(&DEMO_AUDIT)
    .error_codes(&DEMO_ERRORS)
}

#[derive(serde::Deserialize)]
struct PurgeQuery {
    #[serde(default)]
    locked: bool,
}

/// Route admin de l'extension, gardée par sa propre permission.
async fn purge_notes(
    State(iam): State<IamState>,
    session: AdminSession,
    Query(query): Query<PurgeQuery>,
) -> Result<StatusCode, ApiError> {
    session.require(&iam, Permission::of(NOTES_PURGE)).await?;
    if query.locked {
        return Err(Problem::new(StatusCode::CONFLICT, ErrorCode::of(&DEMO_ERRORS[0])).into());
    }
    // Écriture auditée avec l'action de l'extension, dans la transaction de Kuro.
    let mut tx = AuditedTx::begin(&iam.pool, &session.context)
        .await
        .map_err(|e| ApiError::from(internal_error(&e, "demo purge")))?;
    sqlx::query("delete from demo.notes")
        .execute(&mut *tx)
        .await
        .map_err(|e| ApiError::from(internal_error(&e, "demo purge")))?;
    tx.commit(AuditEvent::success(AuditAction::of(&DEMO_AUDIT[0])).target("notes", "all"))
        .await
        .map_err(|e| ApiError::from(internal_error(&e, "demo purge")))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn count_notes(State(state): State<AppState>) -> Json<Value> {
    let count: i64 = sqlx::query_scalar("select count(*) from demo.notes")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    Json(json!({ "notes": count }))
}

fn no_config() -> BTreeMap<String, Value> {
    BTreeMap::new()
}

async fn table_schema(pool: &PgPool, table: &str) -> Vec<String> {
    sqlx::query_scalar(
        "select table_schema::text from information_schema.tables where table_name = $1 order by 1",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn mounts_the_routes_of_an_extension_beside_those_of_kuro() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;

    let notes = kuro.raw(Method::GET, "/api/demo/v1/notes", &[], None).await;
    assert_eq!(notes.status, StatusCode::OK);
    assert_eq!(notes.json, json!({ "notes": 0 }));
    // Mêmes couches que Kuro : identifiant de requête renvoyé.
    assert!(notes.headers.contains_key("x-request-id"));

    let health = kuro.raw(Method::GET, "/api/health", &[], None).await;
    assert_eq!(health.status, StatusCode::OK);
}

#[tokio::test]
async fn kuro_alone_has_no_extension_route() {
    let kuro = TestAdmin::start().await;

    let response = kuro.raw(Method::GET, "/api/demo/v1/notes", &[], None).await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn passes_its_configuration_section_to_the_extension() {
    let config = BTreeMap::from([("demo".to_owned(), json!({ "greeting": "salut" }))]);
    let kuro = TestAdmin::with_extensions(config, vec![demo()]).await;

    let response = kuro
        .raw(Method::GET, "/api/demo/v1/greeting", &[], None)
        .await;

    assert_eq!(response.json, json!({ "greeting": "salut" }));
}

/// Tables de l'extension dans son schéma, avec sa propre table de migrations : celle de Kuro
/// n'en garde aucune trace.
#[tokio::test]
async fn migrates_the_extension_in_its_own_schema() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;

    assert_eq!(table_schema(&kuro.pool, "notes").await, ["demo"]);
    let demo_versions: Vec<i64> =
        sqlx::query_scalar("select version from demo._sqlx_migrations order by version")
            .fetch_all(&kuro.pool)
            .await
            .unwrap();
    assert_eq!(demo_versions, [1, 2]);
    let kuro_versions: i64 = sqlx::query_scalar(
        "select count(*) from public._sqlx_migrations where description in ('notes', 'tags')",
    )
    .fetch_one(&kuro.pool)
    .await
    .unwrap();
    assert_eq!(kuro_versions, 0);
}

/// Une table d'extension peut viser celles de Kuro : la note d'un serveur supprimé disparaît.
#[tokio::test]
async fn extension_tables_can_reference_kuro_tables() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;

    let error = sqlx::query("insert into demo.notes (server_id, body) values ($1, 'x')")
        .bind(uuid::Uuid::now_v7())
        .execute(&kuro.pool)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("foreign key"), "{error}");
}

/// Redémarrer ne rejoue rien ; le chemin de recherche posé pour migrer ne fuit pas dans le pool.
#[tokio::test]
async fn preparing_again_is_idempotent_and_leaves_the_pool_untouched() {
    let pool = db::fresh_pool().await;
    extension::prepare(&pool, &no_config(), vec![demo()])
        .await
        .unwrap();
    extension::prepare(&pool, &no_config(), vec![demo()])
        .await
        .unwrap();

    for _ in 0..5 {
        let path: String = sqlx::query_scalar("show search_path")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(path, "\"$user\", public");
    }
}

#[tokio::test]
async fn refuses_a_duplicate_extension_before_touching_the_database() {
    let pool = db::fresh_pool().await;

    let result = extension::prepare(&pool, &no_config(), vec![demo(), demo()]).await;

    assert!(matches!(result, Err(ExtensionError::Duplicate(name)) if name == "demo"));
    assert!(table_schema(&pool, "notes").await.is_empty());
}

#[tokio::test]
async fn refuses_a_name_that_would_shadow_kuro_routes() {
    let pool = db::fresh_pool().await;
    let admin = ExtensionFactory::new("admin", |_| Ok(Extension::default()));

    let result = extension::prepare(&pool, &no_config(), vec![admin]).await;

    assert!(matches!(result, Err(ExtensionError::ReservedName(_))));
}

// Permissions d'extension (tranche 5).

#[tokio::test]
async fn the_catalog_lists_extension_permissions_in_their_category() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;
    let root = kuro.super_admin("root@example.com").await;
    let token = kuro.login(&root).await.access_token;

    let catalog = kuro
        .call(Method::GET, "/permissions", Some(&token), None)
        .await;

    let extensions = catalog
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|category| category["key"] == "extensions")
        .expect("extensions category");
    let purge = extensions["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["key"] == "demo.notes.purge")
        .expect("demo permission");
    assert_eq!(purge["label"], "Purger les notes");
    assert_eq!(purge["sensitive"], true);
    assert_eq!(purge["availability"], "available");
}

#[tokio::test]
async fn a_role_grants_an_extension_permission_and_the_extension_checks_it() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;
    let root = kuro.super_admin("root@example.com").await;
    let root = kuro.login(&root).await.access_token;
    let created = kuro
        .call(
            Method::POST,
            "/roles",
            Some(&root),
            Some(json!({ "name": "notes", "permissions": ["demo.notes.purge"] })),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);

    let allowed = kuro
        .account("allowed@example.com", &[Permission::of(NOTES_PURGE)])
        .await;
    let other = kuro
        .account("other@example.com", &[Permission::ServersRead])
        .await;
    let allowed = kuro.login(&allowed).await.access_token;
    let other = kuro.login(&other).await.access_token;
    let purge = |token: String| {
        let kuro = &kuro;
        async move {
            let authorization = format!("Bearer {token}");
            kuro.raw(
                Method::POST,
                "/api/demo/v1/admin/notes/purge",
                &[("authorization", authorization.as_str())],
                None,
            )
            .await
        }
    };

    assert_eq!(purge(allowed).await.status, StatusCode::NO_CONTENT);
    assert_eq!(purge(root).await.status, StatusCode::NO_CONTENT);
    let refused = purge(other).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json["params"]["permission"], "demo.notes.purge");
    let denied = kuro.last_audit("access.denied").await;
    assert_eq!(denied.details["permission"], "demo.notes.purge");
}

#[tokio::test]
async fn the_openapi_document_publishes_registered_extension_permissions() {
    extension::register(&[demo()]).unwrap();

    let document = serde_json::to_value(kuro_api::http::openapi::document()).unwrap();

    let keys = document["components"]["schemas"]["PermissionKey"]["enum"]
        .as_array()
        .unwrap();
    assert!(keys.contains(&json!("demo.notes.purge")));
}

#[tokio::test]
async fn refuses_extension_permissions_outside_its_prefix() {
    static STRAY: [PermissionDef; 1] = [PermissionDef {
        key: "other.notes.purge",
        label: "x",
        sensitive: false,
        agent_grantable: false,
    }];
    let pool = db::fresh_pool().await;
    let stray = ExtensionFactory::new("stray", |_| Ok(Extension::default())).permissions(&STRAY);

    let result = extension::prepare(&pool, &no_config(), vec![stray]).await;

    assert!(matches!(result, Err(ExtensionError::Permission(_))));
}

#[tokio::test]
async fn refuses_an_extension_named_after_a_kuro_permission_domain() {
    let pool = db::fresh_pool().await;
    let servers = ExtensionFactory::new("servers", |_| Ok(Extension::default()));

    let result = extension::prepare(&pool, &no_config(), vec![servers]).await;

    assert!(matches!(result, Err(ExtensionError::ReservedName(_))));
}

// Actions d'audit et codes d'erreur d'extension.

async fn purge(kuro: &TestAdmin, query: &str) -> crate::support::http::TestResponse {
    let root = kuro.super_admin("root@example.com").await;
    let token = kuro.login(&root).await.access_token;
    let authorization = format!("Bearer {token}");
    kuro.raw(
        Method::POST,
        &format!("/api/demo/v1/admin/notes/purge{query}"),
        &[("authorization", authorization.as_str())],
        None,
    )
    .await
}

#[tokio::test]
async fn an_extension_audits_its_writes_with_its_own_action() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;

    let response = purge(&kuro, "").await;

    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.json);
    let event = kuro.last_audit("demo.notes.purged").await;
    assert_eq!(event.outcome, "success");
    assert_eq!(event.actor_kind, "admin");
    assert_eq!(event.target_id.as_deref(), Some("all"));
}

#[tokio::test]
async fn an_extension_answers_with_its_own_error_code() {
    let kuro = TestAdmin::with_extensions(no_config(), vec![demo()]).await;

    let response = purge(&kuro, "?locked=true").await;

    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json["code"], "demo.notes.locked");
    assert_eq!(response.json["title"], "These notes are locked");
}

#[tokio::test]
async fn the_openapi_document_publishes_extension_error_codes() {
    extension::register(&[demo()]).unwrap();

    let document = serde_json::to_value(kuro_api::http::openapi::document()).unwrap();

    let codes = document["components"]["schemas"]["ErrorCode"]["enum"]
        .as_array()
        .unwrap();
    assert!(codes.contains(&json!("demo.notes.locked")));
}

#[tokio::test]
async fn refuses_audit_actions_and_error_codes_outside_its_prefix() {
    static STRAY_ACTIONS: [AuditActionDef; 1] = [AuditActionDef {
        code: "server.created",
    }];
    static STRAY_CODES: [ErrorCodeDef; 1] = [ErrorCodeDef {
        code: "not_found",
        title: "x",
    }];
    let pool = db::fresh_pool().await;
    let actions =
        ExtensionFactory::new("stray", |_| Ok(Extension::default())).audit_actions(&STRAY_ACTIONS);
    let codes =
        ExtensionFactory::new("stray", |_| Ok(Extension::default())).error_codes(&STRAY_CODES);

    let actions = extension::prepare(&pool, &no_config(), vec![actions]).await;
    let codes = extension::prepare(&pool, &no_config(), vec![codes]).await;

    assert!(matches!(actions, Err(ExtensionError::AuditAction(_))));
    assert!(matches!(codes, Err(ExtensionError::ErrorCode(_))));
}
