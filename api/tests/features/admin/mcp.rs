//! Serveur MCP (ADR-0023) : authentification des agents, outils filtrés par droits, refus et
//! actions journalisés comme ceux d'un agent, au nom de l'admin qui a délégué.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use super::oauth::{Fixture, approve, exchange, fixture, param, register, request_id, set_enabled};
use crate::support::http::TestResponse;

/// Jeton d'accès d'un agent à qui l'admin du fixture accorde `scope` ; rend aussi son client.
async fn agent(fixture: &Fixture, scope: &str) -> (String, String) {
    let client_id = register(&fixture.admin).await;
    let request = request_id(&fixture.admin, &client_id, scope).await;
    let code = param(&approve(fixture, &request, None).await, "code").unwrap();
    let tokens = exchange(&fixture.admin, &client_id, &code).await;
    (
        tokens.json["access_token"].as_str().unwrap().to_owned(),
        client_id,
    )
}

/// Requête JSON-RPC sur `/mcp` (protocole 2025-11-25, le plus répandu chez les clients).
async fn rpc(fixture: &Fixture, token: Option<&str>, method: &str, params: Value) -> TestResponse {
    let authorization = token.map(|token| format!("Bearer {token}"));
    let mut headers = vec![
        ("accept", "application/json, text/event-stream"),
        ("mcp-protocol-version", "2025-11-25"),
        ("host", "kuro.test"),
    ];
    if let Some(authorization) = authorization.as_deref() {
        headers.push(("authorization", authorization));
    }
    fixture
        .admin
        .raw(
            Method::POST,
            "/mcp",
            &headers,
            Some((
                "application/json",
                json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
                    .to_string(),
            )),
        )
        .await
}

/// Résultat JSON-RPC, après la poignée de main `initialize`.
async fn call(fixture: &Fixture, token: &str, method: &str, params: Value) -> Value {
    let init = rpc(
        fixture,
        Some(token),
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1.0" },
        }),
    )
    .await;
    assert_eq!(init.status, StatusCode::OK, "{}", init.body);
    let response = rpc(fixture, Some(token), method, params).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert!(response.json["error"].is_null(), "{}", response.body);
    response.json["result"].clone()
}

async fn tool(fixture: &Fixture, token: &str, name: &str, arguments: Value) -> Value {
    call(
        fixture,
        token,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await
}

fn names(tools: &Value) -> Vec<&str> {
    tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn an_agent_without_a_valid_token_is_sent_to_the_oauth_flow() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let response = rpc(&fixture, None, "tools/list", json!({})).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    let challenge = response.headers["www-authenticate"].to_str().unwrap();
    assert!(
        challenge.contains(
            r#"resource_metadata="https://kuro.test/.well-known/oauth-protected-resource/mcp""#
        ),
        "{challenge}"
    );
    let forged = rpc(&fixture, Some("kuro_at_forged"), "tools/list", json!({})).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    // Coupé dans les paramètres : `/mcp` n'existe plus, même avec un jeton valide.
    let (token, _) = agent(&fixture, "servers.read").await;
    set_enabled(&fixture.admin, &fixture.token, false).await;
    let off = rpc(&fixture, Some(&token), "tools/list", json!({})).await;
    assert_eq!(off.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_agent_sees_only_the_tools_its_rights_allow() {
    let fixture = fixture(&[Permission::ServersRead, Permission::PrefabsRead]).await;
    let (token, _) = agent(&fixture, "servers.read").await;

    let tools = call(&fixture, &token, "tools/list", json!({})).await;
    assert_eq!(names(&tools), ["list_servers", "get_server"]);
    assert_eq!(tools["tools"][0]["annotations"]["readOnlyHint"], true);

    let servers = tool(&fixture, &token, "list_servers", json!({})).await;
    assert_eq!(servers["isError"], false);
    assert_eq!(servers["structuredContent"], json!([]));
}

#[tokio::test]
async fn a_tool_beyond_the_grant_is_refused_and_logged_as_the_agent() {
    let fixture = fixture(&[Permission::ServersRead, Permission::ServersPower]).await;
    // L'admin détient `servers.power`, mais ne l'a pas délégué.
    let (token, client_id) = agent(&fixture, "servers.read").await;

    let refused = tool(
        &fixture,
        &token,
        "power_server",
        json!({ "server_id": "01a17a00-0000-7000-8000-0000000000aa", "action": "kill" }),
    )
    .await;
    assert_eq!(refused["isError"], true);
    assert_eq!(refused["structuredContent"]["error"], "forbidden");

    let denied = fixture.admin.last_audit("access.denied").await;
    assert_eq!(denied.actor_kind, "agent");
    assert_eq!(denied.details["permission"], "servers.power");
    assert_eq!(denied.details["tool"], "power_server");
    assert_eq!(denied.details["agent"]["client"], client_id);
}

#[tokio::test]
async fn an_agent_loses_a_right_its_admin_loses() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let (token, _) = agent(&fixture, "servers.read").await;
    assert_eq!(
        names(&call(&fixture, &token, "tools/list", json!({})).await),
        ["list_servers", "get_server"]
    );

    // Rôles de l'admin vidés : l'accord demeure, mais n'ouvre plus rien.
    sqlx::query("DELETE FROM admin_user_roles")
        .execute(&fixture.admin.pool)
        .await
        .unwrap();
    assert!(names(&call(&fixture, &token, "tools/list", json!({})).await).is_empty());
}

#[tokio::test]
async fn tool_errors_carry_a_stable_code() {
    let fixture = fixture(&[Permission::ServersRead, Permission::ServersPower]).await;
    let (token, _) = agent(&fixture, "servers.read servers.power").await;

    let missing = tool(
        &fixture,
        &token,
        "get_server",
        json!({ "server_id": "01a17a00-0000-7000-8000-0000000000aa" }),
    )
    .await;
    assert_eq!(missing["isError"], true);
    assert_eq!(missing["structuredContent"]["error"], "not_found");

    let wrong = tool(
        &fixture,
        &token,
        "power_server",
        json!({ "server_id": "01a17a00-0000-7000-8000-0000000000aa", "action": "explode" }),
    )
    .await;
    assert_eq!(wrong["structuredContent"]["error"], "invalid_arguments");
}

#[tokio::test]
async fn an_agent_builds_a_prefab_in_the_studio() {
    let fixture = fixture(&[Permission::PrefabsRead, Permission::PrefabsManage]).await;
    let (token, client_id) = agent(&fixture, "prefabs.read prefabs.manage").await;

    let schema = tool(&fixture, &token, "get_prefab_schema", json!({})).await;
    assert!(schema["structuredContent"]["properties"]["services"].is_object());

    let created = tool(&fixture, &token, "create_prefab_draft", json!({})).await;
    assert_eq!(created["isError"], false, "{created}");
    let draft = created["structuredContent"]["draft_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut files = created["structuredContent"]["files"].clone();
    assert!(files["prefab.yaml"].is_string());
    let event = fixture.admin.last_audit("prefab_draft.created").await;
    assert_eq!(event.actor_kind, "agent");
    assert_eq!(event.details["agent"]["client"], client_id);

    // Manifest cassé : la validation le dit, sans rien écrire.
    files["prefab.yaml"] = json!("schema: kuro.prefab/v1\nid: [");
    let check = tool(
        &fixture,
        &token,
        "validate_prefab_files",
        json!({ "files": files }),
    )
    .await;
    assert_eq!(check["structuredContent"]["valid"], false);

    let saved = tool(
        &fixture,
        &token,
        "save_prefab_draft",
        json!({ "draft_id": draft, "files": files }),
    )
    .await;
    assert_eq!(saved["structuredContent"]["check"]["valid"], false);
    let reread = tool(
        &fixture,
        &token,
        "get_prefab_draft",
        json!({ "draft_id": draft }),
    )
    .await;
    assert_eq!(
        reread["structuredContent"]["files"]["prefab.yaml"],
        files["prefab.yaml"]
    );
    assert_eq!(
        fixture
            .admin
            .last_audit("prefab_draft.updated")
            .await
            .actor_kind,
        "agent"
    );
}

#[tokio::test]
async fn a_web_page_other_than_the_panel_cannot_call_mcp() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let (token, _) = agent(&fixture, "servers.read").await;
    let authorization = format!("Bearer {token}");
    let from = |origin: &'static str| {
        let authorization = authorization.clone();
        let admin = &fixture.admin;
        async move {
            admin
                .raw(
                    Method::POST,
                    "/mcp",
                    &[
                        ("accept", "application/json, text/event-stream"),
                        ("host", "kuro.test"),
                        ("origin", origin),
                        ("authorization", &authorization),
                    ],
                    Some((
                        "application/json",
                        json!({
                            "jsonrpc": "2.0", "id": 1, "method": "initialize",
                            "params": {
                                "protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": { "name": "test", "version": "1.0" },
                            },
                        })
                        .to_string(),
                    )),
                )
                .await
                .status
        }
    };
    assert_eq!(from("https://evil.example").await, StatusCode::FORBIDDEN);
    assert_eq!(from("https://panel.kuro.test").await, StatusCode::OK);
}
