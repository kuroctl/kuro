//! Serveur d'autorisation du MCP (ADR-0023) : bascule dans les paramètres, enregistrement des
//! clients, consentement, PKCE, rotation et révocation des jetons, droits effectifs d'un agent.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use kuro_api::oauth::application::authenticate;
use serde_json::{Value, json};

use crate::support::admin::TestAdmin;
use crate::support::oauth::ISSUER;

/// Exemple de l'annexe B de la RFC 7636.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const REDIRECT: &str = "http://127.0.0.1:33418/callback";
const CIMD: &str = "https://agent.example/oauth/client.json";
const FORM: &str = "application/x-www-form-urlencoded";

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn form(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={}", encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Paramètre décodé d'une URL de redirection.
pub(super) fn param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| {
            let bytes = value.as_bytes();
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    out.push(u8::from_str_radix(&value[i + 1..i + 3], 16).unwrap());
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).unwrap()
        })
    })
}

pub(super) struct Fixture {
    pub(super) admin: TestAdmin,
    pub(super) token: String,
}

/// Admin aux droits donnés, MCP activé dans les paramètres.
pub(super) async fn fixture(permissions: &[Permission]) -> Fixture {
    let admin = TestAdmin::start().await;
    let mut permissions = permissions.to_vec();
    permissions.push(Permission::SystemSettingsUpdate);
    let account = admin.account("boss@example.com", &permissions).await;
    let token = admin.login(&account).await.access_token;
    set_enabled(&admin, &token, true).await;
    Fixture { admin, token }
}

pub(super) async fn set_enabled(admin: &TestAdmin, token: &str, enabled: bool) {
    let response = admin
        .call(
            Method::PATCH,
            "/settings",
            Some(token),
            Some(json!({ "mcp_enabled": enabled })),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json["mcp_enabled"], enabled);
}

/// Client enregistré dynamiquement ; rend son `client_id`.
pub(super) async fn register(admin: &TestAdmin) -> String {
    let response = admin
        .raw(
            Method::POST,
            "/oauth/register",
            &[],
            Some((
                "application/json",
                json!({ "client_name": "Agent de test", "redirect_uris": [REDIRECT] }).to_string(),
            )),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    assert_eq!(response.json["token_endpoint_auth_method"], "none");
    response.json["client_id"].as_str().unwrap().to_owned()
}

/// `GET /oauth/authorize` ; rend la redirection.
async fn authorize(admin: &TestAdmin, params: &[(&str, &str)]) -> String {
    let response = admin
        .raw(
            Method::GET,
            &format!("/oauth/authorize?{}", form(params)),
            &[],
            None,
        )
        .await;
    assert!(response.status.is_redirection(), "{}", response.status);
    response.headers["location"].to_str().unwrap().to_owned()
}

fn authorize_params<'a>(client_id: &'a str, scope: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", REDIRECT),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S256"),
        ("state", "xyz"),
        ("scope", scope),
        ("resource", "https://kuro.test/mcp"),
    ]
}

/// Demande enregistrée : identifiant lu dans la redirection vers le panel.
pub(super) async fn request_id(admin: &TestAdmin, client_id: &str, scope: &str) -> String {
    let location = authorize(admin, &authorize_params(client_id, scope)).await;
    assert!(
        location.starts_with("https://panel.kuro.test/oauth/consent?request="),
        "{location}"
    );
    param(&location, "request").unwrap()
}

pub(super) async fn approve(fixture: &Fixture, request: &str, scopes: Option<Value>) -> String {
    let body = scopes.map_or(json!({}), |scopes| json!({ "scopes": scopes }));
    let response = fixture
        .admin
        .call(
            Method::POST,
            &format!("/oauth/requests/{request}/approve"),
            Some(&fixture.token),
            Some(body),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    response.json["redirect_to"].as_str().unwrap().to_owned()
}

async fn token(admin: &TestAdmin, params: &[(&str, &str)]) -> crate::support::http::TestResponse {
    admin
        .raw(
            Method::POST,
            "/oauth/token",
            &[],
            Some((FORM, form(params))),
        )
        .await
}

pub(super) async fn exchange(
    admin: &TestAdmin,
    client_id: &str,
    code: &str,
) -> crate::support::http::TestResponse {
    token(
        admin,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
            ("resource", "https://kuro.test/mcp"),
        ],
    )
    .await
}

#[tokio::test]
async fn the_mcp_server_is_switched_on_and_off_live_from_the_settings() {
    let admin = TestAdmin::start().await;
    let account = admin
        .account(
            "boss@example.com",
            &[
                Permission::SystemSettingsRead,
                Permission::SystemSettingsUpdate,
            ],
        )
        .await;
    let session = admin.login(&account).await.access_token;

    let settings = admin
        .call(Method::GET, "/settings", Some(&session), None)
        .await;
    assert_eq!(settings.json["mcp_enabled"], false);
    assert_eq!(settings.json["mcp_url"], "https://kuro.test/mcp");
    let discovery = |path: &'static str| admin.raw(Method::GET, path, &[], None);
    assert_eq!(
        discovery("/.well-known/oauth-authorization-server")
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    set_enabled(&admin, &session, true).await;
    let server = discovery("/.well-known/oauth-authorization-server").await;
    assert_eq!(server.status, StatusCode::OK);
    assert_eq!(server.json["issuer"], ISSUER);
    assert_eq!(
        server.json["code_challenge_methods_supported"],
        json!(["S256"])
    );
    assert_eq!(server.json["client_id_metadata_document_supported"], true);
    let scopes = server.json["scopes_supported"].as_array().unwrap();
    assert!(scopes.contains(&json!("servers.power")));
    assert!(!scopes.contains(&json!("servers.delete")));
    assert!(!scopes.contains(&json!("system.settings.update")));
    let resource = discovery("/.well-known/oauth-protected-resource/mcp").await;
    assert_eq!(resource.json["resource"], "https://kuro.test/mcp");
    assert_eq!(resource.json["authorization_servers"], json!([ISSUER]));

    let changed = admin.last_audit("system.settings_updated").await;
    assert_eq!(
        changed.details["changed"]["mcp_enabled"],
        json!({ "from": false, "to": true })
    );

    set_enabled(&admin, &session, false).await;
    assert_eq!(
        discovery("/.well-known/oauth-protected-resource")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn an_admin_delegates_part_of_their_rights_to_an_agent() {
    let fixture = fixture(&[
        Permission::ServersRead,
        Permission::ServersPower,
        Permission::ServersDelete,
    ])
    .await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;
    let request = request_id(
        admin,
        &client_id,
        "servers.read servers.power servers.delete nodes.read",
    )
    .await;

    // Page de consentement : `servers.delete` n'est jamais accordable, `nodes.read` pas détenu.
    let view = admin
        .call(
            Method::GET,
            &format!("/oauth/requests/{request}"),
            Some(&fixture.token),
            None,
        )
        .await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.body);
    assert_eq!(view.json["client"]["name"], "Agent de test");
    assert_eq!(view.json["client"]["registration"], "dynamic");
    assert_eq!(view.json["client"]["redirect_host"], "127.0.0.1");
    assert_eq!(
        view.json["scopes"],
        json!([
            { "key": "nodes.read", "sensitive": false, "held": false },
            { "key": "servers.read", "sensitive": false, "held": true },
            { "key": "servers.power", "sensitive": false, "held": true },
        ])
    );

    let redirect = approve(&fixture, &request, None).await;
    assert!(redirect.starts_with(REDIRECT), "{redirect}");
    assert_eq!(param(&redirect, "state").as_deref(), Some("xyz"));
    assert_eq!(param(&redirect, "iss").as_deref(), Some(ISSUER));
    let code = param(&redirect, "code").unwrap();
    let approved = admin.last_audit("oauth.grant_approved").await;
    assert_eq!(approved.actor_kind, "admin");
    assert_eq!(
        approved.details["scopes"],
        json!(["servers.read", "servers.power"])
    );

    let tokens = exchange(admin, &client_id, &code).await;
    assert_eq!(tokens.status, StatusCode::OK, "{}", tokens.body);
    assert_eq!(tokens.headers["cache-control"], "no-store");
    assert_eq!(tokens.json["token_type"], "Bearer");
    assert_eq!(tokens.json["expires_in"], 3600);
    assert_eq!(tokens.json["scope"], "servers.read servers.power");
    let access = tokens.json["access_token"].as_str().unwrap().to_owned();
    let issued = admin.last_audit("oauth.token_issued").await;
    assert_eq!(issued.actor_kind, "agent");
    assert!(
        tokens.json["refresh_token"]
            .as_str()
            .unwrap()
            .starts_with("kuro_rt_agent_")
    );
    assert!(!issued.details.to_string().contains(&access));

    // Un code ne sert qu'une fois.
    let again = exchange(admin, &client_id, &code).await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    assert_eq!(again.json["error"], "invalid_grant");

    let agent = authenticate(&admin.pool, &access).await.unwrap().unwrap();
    assert!(
        agent
            .principal
            .permissions
            .contains(Permission::ServersPower)
    );
    assert!(
        !agent
            .principal
            .permissions
            .contains(Permission::ServersDelete)
    );
    assert!(!agent.principal.permissions.contains(Permission::NodesRead));
    assert_eq!(agent.client_id, client_id);
}

#[tokio::test]
async fn a_refresh_token_rotates_and_its_reuse_revokes_the_grant() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;
    let request = request_id(admin, &client_id, "servers.read").await;
    let code = param(&approve(&fixture, &request, None).await, "code").unwrap();
    let first = exchange(admin, &client_id, &code).await;
    let old_refresh = first.json["refresh_token"].as_str().unwrap().to_owned();

    let refresh = |refresh_token: String| {
        let client_id = client_id.clone();
        async move {
            token(
                admin,
                &[
                    ("grant_type", "refresh_token"),
                    ("refresh_token", &refresh_token),
                    ("client_id", &client_id),
                ],
            )
            .await
        }
    };
    let second = refresh(old_refresh.clone()).await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    let access = second.json["access_token"].as_str().unwrap().to_owned();
    assert_ne!(second.json["refresh_token"].as_str().unwrap(), old_refresh);
    assert!(authenticate(&admin.pool, &access).await.unwrap().is_some());

    // Ancien jeton présenté à nouveau : vol présumé, l'accord tombe avec tous ses jetons.
    let reused = refresh(old_refresh).await;
    assert_eq!(reused.json["error"], "invalid_grant");
    assert!(authenticate(&admin.pool, &access).await.unwrap().is_none());
    let event = admin.last_audit("oauth.refresh_reused").await;
    assert_eq!(event.outcome, "failure");
    assert_eq!(event.actor_kind, "agent");
}

#[tokio::test]
async fn a_wrong_pkce_verifier_or_client_gets_no_token() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;
    let other = register(admin).await;

    let request = request_id(admin, &client_id, "servers.read").await;
    let code = param(&approve(&fixture, &request, None).await, "code").unwrap();
    let wrong_client = exchange(admin, &other, &code).await;
    assert_eq!(wrong_client.json["error"], "invalid_grant");

    let request = request_id(admin, &client_id, "servers.read").await;
    let code = param(&approve(&fixture, &request, None).await, "code").unwrap();
    let wrong_verifier = token(
        admin,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", &client_id),
            (
                "code_verifier",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ],
    )
    .await;
    assert_eq!(wrong_verifier.json["error"], "invalid_grant");

    let wrong_resource = token(
        admin,
        &[
            ("grant_type", "authorization_code"),
            ("code", "x"),
            ("client_id", &client_id),
            ("code_verifier", VERIFIER),
            ("resource", "https://elsewhere.example/mcp"),
        ],
    )
    .await;
    assert_eq!(wrong_resource.json["error"], "invalid_target");
}

#[tokio::test]
async fn bad_requests_never_redirect_to_an_unverified_uri() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;

    // URI de retour non déclarée : l'erreur reste dans le panel.
    let mut params = authorize_params(&client_id, "servers.read");
    params[2] = ("redirect_uri", "https://evil.example/callback");
    let location = authorize(admin, &params).await;
    assert_eq!(
        location,
        "https://panel.kuro.test/oauth/consent?error=invalid_redirect_uri"
    );
    let unknown = authorize(admin, &authorize_params("nobody", "servers.read")).await;
    assert!(unknown.ends_with("error=unknown_client"), "{unknown}");

    // Client et URI vérifiés : l'erreur part chez le client, avec `state` et `iss`.
    let mut params = authorize_params(&client_id, "servers.read");
    params[4] = ("code_challenge_method", "plain");
    let location = authorize(admin, &params).await;
    assert!(location.starts_with(REDIRECT), "{location}");
    assert_eq!(
        param(&location, "error").as_deref(),
        Some("invalid_request")
    );
    assert_eq!(param(&location, "state").as_deref(), Some("xyz"));
    assert_eq!(param(&location, "iss").as_deref(), Some(ISSUER));

    let location = authorize(admin, &authorize_params(&client_id, "servers.delete")).await;
    assert_eq!(param(&location, "error").as_deref(), Some("invalid_scope"));
}

#[tokio::test]
async fn a_client_is_known_by_its_metadata_document() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    admin.metadata.serve(
        CIMD,
        json!({
            "client_id": CIMD,
            "client_name": "Agent publié",
            "client_uri": "https://agent.example",
            "redirect_uris": [REDIRECT],
        }),
    );
    let request = request_id(admin, CIMD, "servers.read").await;
    let view = admin
        .call(
            Method::GET,
            &format!("/oauth/requests/{request}"),
            Some(&fixture.token),
            None,
        )
        .await;
    assert_eq!(view.json["client"]["registration"], "metadata");
    assert_eq!(view.json["client"]["client_id"], CIMD);
    assert_eq!(view.json["client"]["uri"], "https://agent.example");

    // Document relu au plus une fois par heure.
    request_id(admin, CIMD, "servers.read").await;
    assert_eq!(*admin.metadata.fetches.lock().unwrap(), 1);

    // Document qui ne se désigne pas lui-même : refusé.
    let other = "https://agent.example/oauth/other.json";
    admin.metadata.serve(
        other,
        json!({ "client_id": CIMD, "client_name": "x", "redirect_uris": [REDIRECT] }),
    );
    let location = authorize(admin, &authorize_params(other, "servers.read")).await;
    assert!(location.ends_with("error=unknown_client"), "{location}");
}

#[tokio::test]
async fn an_admin_refuses_or_grants_nothing_they_do_not_hold() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;

    let request = request_id(admin, &client_id, "nodes.read").await;
    let refused = admin
        .call(
            Method::POST,
            &format!("/oauth/requests/{request}/approve"),
            Some(&fixture.token),
            Some(json!({})),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(refused.json["code"], "oauth_no_scope");

    let denied = admin
        .call(
            Method::POST,
            &format!("/oauth/requests/{request}/deny"),
            Some(&fixture.token),
            None,
        )
        .await;
    let redirect = denied.json["redirect_to"].as_str().unwrap();
    assert_eq!(param(redirect, "error").as_deref(), Some("access_denied"));
    assert_eq!(param(redirect, "state").as_deref(), Some("xyz"));
    admin.last_audit("oauth.grant_denied").await;
    // Demande close : ni consultable, ni accordable.
    let gone = admin
        .call(
            Method::GET,
            &format!("/oauth/requests/{request}"),
            Some(&fixture.token),
            None,
        )
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn revoking_a_grant_or_switching_off_the_mcp_cuts_the_agent_off() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;
    let request = request_id(admin, &client_id, "servers.read").await;
    let code = param(&approve(&fixture, &request, None).await, "code").unwrap();
    let access = exchange(admin, &client_id, &code).await.json["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(authenticate(&admin.pool, &access).await.unwrap().is_some());

    set_enabled(admin, &fixture.token, false).await;
    assert!(authenticate(&admin.pool, &access).await.unwrap().is_none());
    set_enabled(admin, &fixture.token, true).await;
    assert!(authenticate(&admin.pool, &access).await.unwrap().is_some());

    let grants = admin
        .call(Method::GET, "/oauth/grants", Some(&fixture.token), None)
        .await;
    assert_eq!(grants.json.as_array().unwrap().len(), 1);
    assert_eq!(grants.json[0]["client_name"], "Agent de test");
    assert_eq!(grants.json[0]["scopes"], json!(["servers.read"]));
    assert!(!grants.json[0]["last_used_at"].is_null());
    let grant = grants.json[0]["id"].as_str().unwrap().to_owned();

    // Un autre admin ne voit ni ne révoque cet accès.
    let other = admin.account("other@example.com", &[]).await;
    let other_token = admin.login(&other).await.access_token;
    let forbidden = admin
        .call(
            Method::DELETE,
            &format!("/oauth/grants/{grant}"),
            Some(&other_token),
            None,
        )
        .await;
    assert_eq!(forbidden.status, StatusCode::NOT_FOUND);

    let revoked = admin
        .call(
            Method::DELETE,
            &format!("/oauth/grants/{grant}"),
            Some(&fixture.token),
            None,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    assert!(authenticate(&admin.pool, &access).await.unwrap().is_none());
    assert_eq!(
        admin.last_audit("oauth.grant_revoked").await.details["by"],
        "admin"
    );
}

#[tokio::test]
async fn a_client_revokes_its_own_refresh_token() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    let client_id = register(admin).await;
    let request = request_id(admin, &client_id, "servers.read").await;
    let code = param(&approve(&fixture, &request, None).await, "code").unwrap();
    let tokens = exchange(admin, &client_id, &code).await;
    let access = tokens.json["access_token"].as_str().unwrap();
    let refresh = tokens.json["refresh_token"].as_str().unwrap();

    let revoke = |token: &str| {
        admin.raw(
            Method::POST,
            "/oauth/revoke",
            &[],
            Some((FORM, form(&[("token", token), ("client_id", &client_id)]))),
        )
    };
    assert_eq!(revoke(refresh).await.status, StatusCode::OK);
    assert!(authenticate(&admin.pool, access).await.unwrap().is_none());
    // Jeton inconnu : pas une erreur (RFC 7009).
    assert_eq!(revoke("kuro_rt_agent_nope").await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_native_client_comes_back_on_any_loopback_port() {
    let fixture = fixture(&[Permission::ServersRead]).await;
    let admin = &fixture.admin;
    // Comme Claude Code : URI déclarées sans port, retour sur un port libre.
    let url = "https://claude.ai/oauth/claude-code-client-metadata";
    admin.metadata.serve(
        url,
        json!({
            "client_id": url,
            "client_name": "Claude Code",
            "redirect_uris": ["http://localhost/callback", "http://127.0.0.1/callback"],
        }),
    );
    let mut params = authorize_params(url, "servers.read");
    params[2] = ("redirect_uri", "http://localhost:51234/callback");
    let location = authorize(admin, &params).await;
    let request = param(&location, "request").expect(&location);
    let redirect = approve(&fixture, &request, None).await;
    assert!(
        redirect.starts_with("http://localhost:51234/callback?code="),
        "{redirect}"
    );

    // Même port libre, mais un autre chemin : refusé.
    params[2] = ("redirect_uri", "http://localhost:51234/steal");
    let location = authorize(admin, &params).await;
    assert!(
        location.ends_with("error=invalid_redirect_uri"),
        "{location}"
    );
}
