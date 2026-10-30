//! Garde de permission : chaque route exige sa permission, et le refus est journalisé.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::admin::TestAdmin;

/// Toutes les routes protégées de l'espace admin, avec un corps valide.
fn protected_routes() -> Vec<(Method, String, Option<Value>, Vec<Permission>)> {
    let id = Uuid::now_v7();
    vec![
        (
            Method::GET,
            "/users".into(),
            None,
            vec![Permission::IamUsersRead],
        ),
        (
            Method::GET,
            format!("/users/{id}"),
            None,
            vec![Permission::IamUsersRead],
        ),
        (
            Method::POST,
            "/users".into(),
            Some(
                json!({ "email": "new@example.com", "display_name": "New", "password": "a long enough password" }),
            ),
            vec![Permission::IamUsersCreate],
        ),
        (
            Method::PATCH,
            format!("/users/{id}"),
            Some(json!({ "display_name": "X" })),
            vec![Permission::IamUsersUpdate],
        ),
        (
            Method::DELETE,
            format!("/users/{id}"),
            None,
            vec![Permission::IamUsersDelete],
        ),
        (
            Method::PUT,
            format!("/users/{id}/roles"),
            Some(json!({ "role_ids": [] })),
            vec![Permission::IamRolesAssign],
        ),
        (
            Method::PUT,
            format!("/users/{id}/password"),
            Some(json!({ "password": "a long enough password" })),
            vec![Permission::IamUsersCredentials],
        ),
        (
            Method::DELETE,
            format!("/users/{id}/totp"),
            None,
            vec![Permission::IamUsersCredentials],
        ),
        (
            Method::DELETE,
            format!("/users/{id}/sessions"),
            None,
            vec![Permission::IamUsersCredentials],
        ),
        (
            Method::GET,
            "/roles".into(),
            None,
            vec![Permission::IamRolesRead],
        ),
        (
            Method::GET,
            format!("/roles/{id}"),
            None,
            vec![Permission::IamRolesRead],
        ),
        (
            Method::POST,
            "/roles".into(),
            Some(json!({ "name": "support", "permissions": [] })),
            vec![Permission::IamRolesManage],
        ),
        (
            Method::PATCH,
            format!("/roles/{id}"),
            Some(json!({ "description": "x" })),
            vec![Permission::IamRolesManage],
        ),
        (
            Method::DELETE,
            format!("/roles/{id}"),
            None,
            vec![Permission::IamRolesManage],
        ),
        (
            Method::GET,
            "/permissions".into(),
            None,
            vec![Permission::IamRolesRead],
        ),
        (
            Method::GET,
            "/audit".into(),
            None,
            vec![Permission::AuditRead],
        ),
        (
            Method::GET,
            "/regions".into(),
            None,
            vec![Permission::NodesRead],
        ),
        (
            Method::GET,
            format!("/regions/{id}"),
            None,
            vec![Permission::NodesRead],
        ),
        (
            Method::POST,
            "/regions".into(),
            Some(json!({ "slug": "eu", "name": "EU", "latitude": 0.0, "longitude": 0.0 })),
            vec![Permission::NodesRegionsManage],
        ),
        (
            Method::PATCH,
            format!("/regions/{id}"),
            Some(json!({ "name": "X" })),
            vec![Permission::NodesRegionsManage],
        ),
        (
            Method::DELETE,
            format!("/regions/{id}"),
            None,
            vec![Permission::NodesRegionsManage],
        ),
        (
            Method::GET,
            "/nodes?unassigned=false".into(),
            None,
            vec![Permission::NodesRead],
        ),
        (
            Method::GET,
            format!("/nodes/{id}"),
            None,
            vec![Permission::NodesRead],
        ),
        (
            Method::POST,
            "/nodes".into(),
            Some(json!({ "name": "node-x" })),
            vec![Permission::NodesCreate],
        ),
        (
            Method::PATCH,
            format!("/nodes/{id}"),
            Some(json!({ "display_name": "X" })),
            vec![Permission::NodesUpdate],
        ),
        (
            Method::POST,
            format!("/nodes/{id}/enrollment-token"),
            None,
            vec![Permission::NodesEnrollment],
        ),
        (
            Method::POST,
            format!("/nodes/{id}/revoke"),
            Some(json!({})),
            vec![Permission::NodesRevoke],
        ),
        (
            Method::DELETE,
            format!("/nodes/{id}"),
            None,
            vec![Permission::NodesDelete],
        ),
        (
            Method::POST,
            format!("/servers/{id}/upgrade"),
            Some(json!({"to": "x"})),
            vec![Permission::ServersUpgrade],
        ),
        (
            Method::POST,
            "/servers/upgrade".into(),
            Some(json!({"servers": [], "to": "x"})),
            vec![Permission::ServersUpgrade],
        ),
        (
            Method::DELETE,
            format!("/nodes/{id}/allocations/{id}"),
            None,
            vec![Permission::NodesAllocationsManage],
        ),
        (
            Method::DELETE,
            format!("/prefabs/{id}/1.0.0"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::DELETE,
            format!("/servers/{id}"),
            None,
            vec![Permission::ServersDelete],
        ),
        (
            Method::DELETE,
            format!("/servers/{id}/backups/{id}"),
            None,
            vec![Permission::ServersBackupsDelete],
        ),
        (
            Method::DELETE,
            format!("/servers/{id}/content/mods/{id}"),
            None,
            vec![Permission::ServersContentManage],
        ),
        (
            Method::DELETE,
            format!("/servers/{id}/files?path=x"),
            None,
            vec![Permission::ServersFilesWrite],
        ),
        (
            Method::DELETE,
            format!("/servers/{id}/schedules/{id}"),
            None,
            vec![Permission::ServersSchedulesManage],
        ),
        (
            Method::DELETE,
            "/settings/content-sources/curseforge".into(),
            None,
            vec![Permission::SystemContentSourcesManage],
        ),
        (
            Method::DELETE,
            "/settings/dns".into(),
            None,
            vec![Permission::SystemDnsManage],
        ),
        (
            Method::DELETE,
            "/settings/steam-account".into(),
            None,
            vec![Permission::SystemSteamAccountsManage],
        ),
        (
            Method::GET,
            "/audit/export".into(),
            None,
            vec![Permission::AuditRead],
        ),
        (
            Method::GET,
            "/content/loaders".into(),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            "/dns".into(),
            None,
            vec![Permission::ServersRead],
        ),
        (
            Method::GET,
            format!("/nodes/{id}/allocations"),
            None,
            vec![Permission::NodesAllocationsRead],
        ),
        (
            Method::GET,
            "/prefabs".into(),
            None,
            vec![Permission::PrefabsRead],
        ),
        (
            Method::GET,
            format!("/prefabs/{id}/1.0.0"),
            None,
            vec![Permission::PrefabsRead],
        ),
        (
            Method::GET,
            format!("/prefabs/{id}/1.0.0/assets/icon"),
            None,
            vec![Permission::PrefabsRead],
        ),
        (
            Method::GET,
            format!("/prefabs/{id}/1.0.0/locales/fr"),
            None,
            vec![Permission::PrefabsRead],
        ),
        (
            Method::GET,
            format!("/prefabs/{id}/1.0.0/servers"),
            None,
            vec![Permission::PrefabsRead, Permission::ServersRead],
        ),
        (
            Method::GET,
            format!("/prefabs/{id}/1.0.0/upgrades"),
            None,
            vec![Permission::ServersUpgrade],
        ),
        (
            Method::GET,
            "/servers".into(),
            None,
            vec![Permission::ServersRead],
        ),
        (
            Method::GET,
            "/servers/search?all_ids=false".into(),
            None,
            vec![Permission::ServersRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}"),
            None,
            vec![Permission::ServersRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/backups"),
            None,
            vec![Permission::ServersBackupsRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods/categories"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods/modpacks/search"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods/modpacks/x/versions"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods/projects/x"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods/projects/x/versions"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/content/mods/search"),
            None,
            vec![Permission::ServersContentRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/files?path=x"),
            None,
            vec![Permission::ServersFilesRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/files/content?path=x"),
            None,
            vec![Permission::ServersFilesRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/schedules"),
            None,
            vec![Permission::ServersSchedulesRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/schedules/{id}"),
            None,
            vec![Permission::ServersSchedulesRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/schedules/{id}/runs"),
            None,
            vec![Permission::ServersSchedulesRead],
        ),
        (
            Method::GET,
            format!("/servers/{id}/upgrade?to=x"),
            None,
            vec![Permission::ServersUpgrade],
        ),
        (
            Method::GET,
            format!("/servers/{id}/upgrades"),
            None,
            vec![Permission::ServersRead],
        ),
        (
            Method::GET,
            "/settings".into(),
            None,
            vec![Permission::SystemSettingsRead],
        ),
        (
            Method::GET,
            "/settings/content-sources".into(),
            None,
            vec![Permission::SystemContentSourcesManage],
        ),
        (
            Method::GET,
            "/settings/dns".into(),
            None,
            vec![Permission::SystemDnsManage],
        ),
        (
            Method::GET,
            "/settings/steam-account".into(),
            None,
            vec![Permission::SystemSteamAccountsManage],
        ),
        (
            Method::GET,
            "/studio/drafts".into(),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::GET,
            "/studio/schema".into(),
            None,
            vec![Permission::PrefabsRead],
        ),
        (
            Method::PATCH,
            format!("/servers/{id}/backups/{id}"),
            Some(json!({"locked": false})),
            vec![Permission::ServersBackupsCreate],
        ),
        (
            Method::PATCH,
            format!("/servers/{id}/content/mods/{id}"),
            Some(json!({"enabled": false})),
            vec![Permission::ServersContentManage],
        ),
        (
            Method::PATCH,
            format!("/servers/{id}/resources"),
            Some(json!({})),
            vec![Permission::ServersUpdate],
        ),
        (
            Method::PATCH,
            format!("/servers/{id}/settings"),
            Some(json!({"settings": {}})),
            vec![Permission::ServersUpdate],
        ),
        (
            Method::PATCH,
            "/settings".into(),
            Some(json!({})),
            vec![Permission::SystemSettingsUpdate],
        ),
        (
            Method::POST,
            "/audit/verify".into(),
            None,
            vec![Permission::AuditRead],
        ),
        (
            Method::POST,
            format!("/nodes/{id}/allocations"),
            Some(json!({"ports": "x"})),
            vec![Permission::NodesAllocationsManage],
        ),
        (
            Method::POST,
            "/prefabs/import".into(),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            "/prefabs/reload".into(),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            format!("/prefabs/{id}/1.0.0/assets/optimize"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            "/servers".into(),
            Some(
                json!({"name": "x", "node_id": "00000000-0000-7000-8000-000000000000", "prefab_id": "x"}),
            ),
            vec![Permission::ServersCreate],
        ),
        (
            Method::POST,
            "/servers/bulk".into(),
            Some(json!({"action": "start", "servers": []})),
            vec![Permission::ServersPower],
        ),
        (
            Method::POST,
            format!("/servers/{id}/backups"),
            Some(json!({})),
            vec![Permission::ServersBackupsCreate],
        ),
        (
            Method::POST,
            format!("/servers/{id}/backups/{id}/download/ticket"),
            None,
            vec![Permission::ServersBackupsDownload],
        ),
        (
            Method::POST,
            format!("/servers/{id}/backups/{id}/restore"),
            Some(json!({"mode": "overwrite"})),
            vec![Permission::ServersBackupsRestore],
        ),
        (
            Method::POST,
            format!("/servers/{id}/command"),
            Some(json!({"line": "x"})),
            vec![Permission::ServersConsoleWrite],
        ),
        (
            Method::POST,
            format!("/servers/{id}/console/ticket"),
            None,
            vec![Permission::ServersConsoleRead],
        ),
        (
            Method::POST,
            format!("/servers/{id}/content/mods"),
            Some(json!({"version_id": "x"})),
            vec![Permission::ServersContentManage],
        ),
        (
            Method::POST,
            format!("/servers/{id}/content/mods/modpacks"),
            Some(json!({"version_id": "x"})),
            vec![Permission::ServersContentManage],
        ),
        (
            Method::POST,
            format!("/servers/{id}/content/mods/upload?file_name=x"),
            None,
            vec![Permission::ServersContentManage],
        ),
        (
            Method::POST,
            format!("/servers/{id}/content/mods/{id}/update"),
            None,
            vec![Permission::ServersContentManage],
        ),
        (
            Method::POST,
            format!("/servers/{id}/files/directories"),
            Some(json!({"path": "x"})),
            vec![Permission::ServersFilesWrite],
        ),
        (
            Method::POST,
            format!("/servers/{id}/files/rename"),
            Some(json!({"from": "x", "to": "x"})),
            vec![Permission::ServersFilesWrite],
        ),
        (
            Method::POST,
            format!("/servers/{id}/power"),
            Some(json!({"action": "start"})),
            vec![Permission::ServersPower],
        ),
        (
            Method::POST,
            format!("/servers/{id}/reinstall"),
            Some(json!({"mode": "reinstall"})),
            vec![Permission::ServersReinstall],
        ),
        (
            Method::POST,
            format!("/servers/{id}/schedules"),
            Some(json!({"cron": "x", "name": "x", "steps": [], "timezone": "x"})),
            vec![Permission::ServersSchedulesManage],
        ),
        (
            Method::POST,
            format!("/servers/{id}/schedules/preview"),
            Some(json!({"cron": "x", "timezone": "x"})),
            vec![Permission::ServersSchedulesRead],
        ),
        (
            Method::POST,
            format!("/servers/{id}/schedules/{id}/run"),
            None,
            vec![Permission::ServersSchedulesManage],
        ),
        (
            Method::POST,
            format!("/servers/{id}/suspend"),
            None,
            vec![Permission::ServersSuspend],
        ),
        (
            Method::POST,
            format!("/servers/{id}/unsuspend"),
            None,
            vec![Permission::ServersSuspend],
        ),
        (
            Method::POST,
            "/settings/dns/sync".into(),
            None,
            vec![Permission::SystemDnsManage],
        ),
        (
            Method::POST,
            "/settings/dns/test".into(),
            None,
            vec![Permission::SystemDnsManage],
        ),
        (
            Method::POST,
            "/studio/compose".into(),
            Some(json!({"text": "x"})),
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            "/studio/drafts".into(),
            Some(json!({})),
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            format!("/studio/drafts/{id}/publish"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            format!("/studio/drafts/{id}/trial"),
            Some(json!({"name": "x", "node_id": "00000000-0000-7000-8000-000000000000"})),
            vec![Permission::PrefabsManage, Permission::ServersCreate],
        ),
        (
            Method::POST,
            "/studio/egg".into(),
            Some(json!({"text": "x"})),
            vec![Permission::PrefabsManage],
        ),
        (
            Method::POST,
            "/studio/validate".into(),
            Some(json!({"files": {}})),
            vec![Permission::PrefabsManage],
        ),
        (
            Method::PUT,
            format!("/nodes/{id}/address"),
            Some(json!({})),
            vec![Permission::NodesUpdate],
        ),
        (
            Method::PUT,
            format!("/servers/{id}/content/mods/{id}/values"),
            Some(json!({"values": {}})),
            vec![Permission::ServersContentManage],
        ),
        (
            Method::PUT,
            format!("/servers/{id}/dns"),
            Some(json!({})),
            vec![Permission::ServersUpdate],
        ),
        (
            Method::PUT,
            format!("/servers/{id}/files/content"),
            Some(json!({"content": "x", "path": "x"})),
            vec![Permission::ServersFilesWrite],
        ),
        (
            Method::PUT,
            format!("/servers/{id}/files/upload?path=x"),
            None,
            vec![Permission::ServersFilesWrite],
        ),
        (
            Method::PUT,
            format!("/servers/{id}/schedules/{id}"),
            Some(json!({"cron": "x", "name": "x", "steps": [], "timezone": "x"})),
            vec![Permission::ServersSchedulesManage],
        ),
        (
            Method::PUT,
            "/settings/content-sources/curseforge".into(),
            Some(json!({"api_key": "x"})),
            vec![Permission::SystemContentSourcesManage],
        ),
        (
            Method::PUT,
            "/settings/dns".into(),
            Some(json!({"base_domain": "x", "provider": "x"})),
            vec![Permission::SystemDnsManage],
        ),
        (
            Method::PUT,
            "/settings/steam-account".into(),
            Some(json!({"password": "x", "username": "x"})),
            vec![Permission::SystemSteamAccountsManage],
        ),
        (
            Method::PUT,
            format!("/studio/drafts/{id}"),
            Some(json!({"files": {}})),
            vec![Permission::PrefabsManage],
        ),
        (
            Method::PUT,
            format!("/studio/drafts/{id}/assets/icon"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::GET,
            format!("/studio/drafts/{id}"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::DELETE,
            format!("/studio/drafts/{id}"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::GET,
            format!("/studio/drafts/{id}/assets/icon"),
            None,
            vec![Permission::PrefabsManage],
        ),
        (
            Method::DELETE,
            format!("/studio/drafts/{id}/assets/icon"),
            None,
            vec![Permission::PrefabsManage],
        ),
    ]
}

#[tokio::test]
async fn every_protected_route_refuses_an_account_without_permission() {
    let admin = TestAdmin::start().await;
    let account = admin.account("nobody@example.com", &[]).await;
    let session = admin.login(&account).await;

    for (method, path, body, permissions) in protected_routes() {
        let response = admin
            .call(method.clone(), &path, Some(&session.access_token), body)
            .await;

        assert_eq!(response.status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(response.json["code"], "forbidden");
        assert_eq!(
            response.json["params"]["permission"],
            permissions[0].key(),
            "{method} {path}"
        );
    }

    let denials: Vec<_> = admin
        .audit()
        .await
        .into_iter()
        .filter(|row| row.action == "access.denied")
        .collect();
    assert_eq!(denials.len(), protected_routes().len());
    assert!(
        denials
            .iter()
            .all(|row| row.outcome == "denied" && row.actor_id == Some(account.id))
    );
}

#[tokio::test]
async fn each_route_opens_with_exactly_its_permission() {
    let admin = TestAdmin::start().await;

    for (index, (method, path, body, permissions)) in protected_routes().into_iter().enumerate() {
        let account = admin
            .account(&format!("user{index}@example.com"), &permissions)
            .await;
        let session = admin.login(&account).await;

        let response = admin
            .call(method.clone(), &path, Some(&session.access_token), body)
            .await;

        assert_ne!(
            response.status,
            StatusCode::FORBIDDEN,
            "{method} {path}: {}",
            response.json
        );
        assert_ne!(response.status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

#[tokio::test]
async fn removing_a_role_takes_effect_on_the_next_request() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let root_session = admin.login(&root).await;
    let reader = admin
        .account("reader@example.com", &[Permission::AuditRead])
        .await;
    let reader_session = admin.login(&reader).await;
    assert_eq!(
        admin
            .call(
                Method::GET,
                "/audit",
                Some(&reader_session.access_token),
                None
            )
            .await
            .status,
        StatusCode::OK
    );

    let unassign = admin
        .call(
            Method::PUT,
            &format!("/users/{}/roles", reader.id),
            Some(&root_session.access_token),
            Some(json!({ "role_ids": [] })),
        )
        .await;
    assert_eq!(unassign.status, StatusCode::OK, "{}", unassign.json);

    let after = admin
        .call(
            Method::GET,
            "/audit",
            Some(&reader_session.access_token),
            None,
        )
        .await;
    assert_eq!(after.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn disabling_an_account_cuts_its_sessions_immediately() {
    let admin = TestAdmin::start().await;
    let root = admin.super_admin("root@example.com").await;
    let root_session = admin.login(&root).await;
    let target = admin.account("target@example.com", &[]).await;
    let target_session = admin.login(&target).await;

    let disable = admin
        .call(
            Method::PATCH,
            &format!("/users/{}", target.id),
            Some(&root_session.access_token),
            Some(json!({ "disabled": true })),
        )
        .await;

    assert_eq!(disable.status, StatusCode::OK, "{}", disable.json);
    let me = admin
        .call(Method::GET, "/me", Some(&target_session.access_token), None)
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        admin
            .login_with(&target.email, crate::support::admin::PASSWORD)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        admin.last_audit("iam.user.disabled").await.actor_id,
        Some(root.id)
    );
}

/// Routes de l'espace admin protégées par une session mais sans permission du catalogue :
/// elles n'agissent que sur le compte ou les consentements de l'appelant, ou sur un ticket.
const SESSION_ONLY_ROUTES: &[(&str, &str)] = &[
    ("GET", "/me"),
    ("PUT", "/me/password"),
    ("POST", "/auth/logout"),
    ("GET", "/oauth/grants"),
    ("DELETE", "/oauth/grants/{id}"),
    ("GET", "/oauth/requests/{id}"),
    ("POST", "/oauth/requests/{id}/approve"),
    ("POST", "/oauth/requests/{id}/deny"),
    ("POST", "/servers/{id}/backups/{backup_id}/download"),
];

/// Forme d'une route, paramètres effacés : `/servers/{}/files`.
fn shape(path: &str) -> String {
    let uuid = |segment: &str| {
        segment.len() == 36 && segment.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    };
    path.split('?')
        .next()
        .unwrap_or_default()
        .split('/')
        .map(|segment| {
            let placeholder = segment.starts_with('{')
                || uuid(segment)
                || matches!(
                    segment,
                    "mods" | "x" | "1.0.0" | "fr" | "curseforge" | "icon"
                );
            if placeholder { "{}" } else { segment }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Une route admin ajoutée sans entrée dans la matrice fait échouer ce test : chaque opération
/// `bearer` de l'OpenAPI est couverte, ou déclarée comme ne relevant que de la session.
#[test]
fn the_matrix_covers_every_secured_admin_route_of_the_openapi_document() {
    let document: Value =
        serde_json::from_str(&kuro_api::http::openapi::to_json().unwrap()).unwrap();
    let covered: std::collections::BTreeSet<(String, String)> = protected_routes()
        .into_iter()
        .map(|(method, path, _, _)| (method.to_string(), shape(&path)))
        .collect();
    let session_only: std::collections::BTreeSet<(String, String)> = SESSION_ONLY_ROUTES
        .iter()
        .map(|(method, path)| ((*method).to_owned(), shape(path)))
        .collect();

    let mut missing = Vec::new();
    for (path, operations) in document["paths"].as_object().unwrap() {
        let Some(short) = path.strip_prefix("/api/admin/v1") else {
            continue;
        };
        for (method, operation) in operations.as_object().unwrap() {
            if operation["security"].is_null() {
                continue;
            }
            let key = (method.to_uppercase(), shape(short));
            if !covered.contains(&key) && !session_only.contains(&key) {
                missing.push(format!("{} {short}", method.to_uppercase()));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "secured admin routes without a matrix entry: {missing:?}"
    );
}
