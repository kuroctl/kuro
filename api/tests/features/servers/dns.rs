//! DNS des serveurs et des nœuds (ADR-0026) : zone configurée avec un jeton vérifié, noms des
//! nœuds et des serveurs, SRV déclaré par le prefab, renommage, réparation des écarts, panne du
//! fournisseur, permissions.

use axum::http::{Method, StatusCode};
use kuro_api::dns::application::reconcile;
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};

use crate::support::dns::TOKEN;
use crate::support::platform::{FakeDaemon, Platform};

fn rec(kind: &str, name: &str, content: &str) -> (String, String, String) {
    (kind.to_owned(), name.to_owned(), content.to_owned())
}

async fn configure(platform: &Platform, body: Value) -> crate::support::http::TestResponse {
    platform
        .call(Method::PUT, "/settings/dns", Some(body))
        .await
}

/// Zone configurée, nœud avec son adresse, serveur Minecraft déployé.
async fn setup() -> (Platform, FakeDaemon, String, u64) {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let zone = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "play.kuroctl.test", "token": TOKEN }),
    )
    .await;
    assert_eq!(zone.status, StatusCode::OK, "{}", zone.json);
    let address = platform
        .call(
            Method::PUT,
            &format!("/nodes/{}/address", daemon.node.id.as_uuid()),
            Some(json!({ "ipv4": "203.0.113.7" })),
        )
        .await;
    assert_eq!(address.status, StatusCode::NO_CONTENT, "{}", address.json);
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let port = server["allocations"][0]["port"].as_u64().unwrap();
    (
        platform,
        daemon,
        server["id"].as_str().unwrap().to_owned(),
        port,
    )
}

#[tokio::test]
async fn a_zone_is_saved_only_when_the_provider_accepts_the_token() {
    let platform = Platform::start().await;
    let refused = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "play.kuroctl.test", "token": "nope" }),
    )
    .await;
    assert_eq!(refused.json["code"], "dns_provider_unauthorized");
    let elsewhere = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "play.example.org", "token": TOKEN }),
    )
    .await;
    assert_eq!(elsewhere.json["code"], "dns_zone_not_found");
    let invalid = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "not a domain", "token": TOKEN }),
    )
    .await;
    assert_eq!(invalid.json["code"], "invalid_domain");

    let saved = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "Play.Kuroctl.Test.", "token": TOKEN }),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(saved.json["base_domain"], "play.kuroctl.test");
    assert_eq!(saved.json["has_token"], true);
    assert!(!saved.body.contains(TOKEN));
    let event = platform.admin.last_audit("dns.zone_updated").await;
    assert!(!event.details.to_string().contains(TOKEN));

    // Sans nouveau jeton, celui enregistré est gardé.
    let ttl = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "play.kuroctl.test", "ttl": 120 }),
    )
    .await;
    assert_eq!(ttl.json["ttl"], 120);
    let test = platform
        .call(Method::POST, "/settings/dns/test", None)
        .await;
    assert_eq!(test.status, StatusCode::OK, "{}", test.json);

    let public = platform.call(Method::GET, "/dns", None).await;
    assert_eq!(public.json["base_domain"], "play.kuroctl.test");
}

#[tokio::test]
async fn nodes_and_servers_get_their_names_and_the_declared_srv() {
    let (platform, _daemon, id, port) = setup().await;
    reconcile(&platform.admin.dns).await.unwrap();

    assert_eq!(
        platform.admin.dns_provider.published(),
        [
            rec(
                "SRV",
                "_minecraft._tcp.survie.play.kuroctl.test",
                &format!("0 0 {port} node-1.nodes.play.kuroctl.test"),
            ),
            rec("A", "node-1.nodes.play.kuroctl.test", "203.0.113.7"),
            rec(
                "CNAME",
                "survie.play.kuroctl.test",
                "node-1.nodes.play.kuroctl.test"
            ),
        ]
    );
    let server = platform
        .call(Method::GET, &format!("/servers/{id}"), None)
        .await;
    assert_eq!(server.json["dns_label"], "survie");
    let created = platform.admin.last_audit("dns.record_created").await;
    assert_eq!(created.actor_kind, "system");

    // Rien à refaire tant que rien ne change.
    let again = reconcile(&platform.admin.dns).await.unwrap();
    assert_eq!(again, Default::default());
}

#[tokio::test]
async fn renaming_a_server_moves_its_records_and_names_stay_unique() {
    let (platform, mut daemon, id, _) = setup().await;
    reconcile(&platform.admin.dns).await.unwrap();

    let renamed = platform
        .call(
            Method::PUT,
            &format!("/servers/{id}/dns"),
            Some(json!({ "label": "creatif" })),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::NO_CONTENT, "{}", renamed.json);
    reconcile(&platform.admin.dns).await.unwrap();
    let names: Vec<String> = platform
        .admin
        .dns_provider
        .published()
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    assert!(names.contains(&"creatif.play.kuroctl.test".to_owned()));
    assert!(!names.iter().any(|n| n.contains("survie")));

    // Un second serveur du même nom reçoit une variante ; un label pris est refusé.
    let (second, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    reconcile(&platform.admin.dns).await.unwrap();
    let second_id = second["id"].as_str().unwrap();
    let fetched = platform
        .call(Method::GET, &format!("/servers/{second_id}"), None)
        .await;
    assert_eq!(fetched.json["dns_label"], "survie");
    let taken = platform
        .call(
            Method::PUT,
            &format!("/servers/{second_id}/dns"),
            Some(json!({ "label": "creatif" })),
        )
        .await;
    assert_eq!(taken.json["code"], "dns_label_taken");
    let invalid = platform
        .call(
            Method::PUT,
            &format!("/servers/{second_id}/dns"),
            Some(json!({ "label": "Pas valide" })),
        )
        .await;
    assert_eq!(invalid.json["code"], "invalid_dns_label");
}

#[tokio::test]
async fn a_record_deleted_by_hand_is_recreated_and_foreign_ones_are_left_alone() {
    let (platform, _daemon, _id, _) = setup().await;
    let provider = &platform.admin.dns_provider;
    // Enregistrement créé à la main dans la zone, hors de Kuro.
    provider.records.lock().unwrap().insert(
        "manual".to_owned(),
        kuro_api::dns::domain::Desired {
            kind: kuro_api::dns::domain::RecordKind::A,
            name: "www.kuroctl.test".to_owned(),
            content: "198.51.100.1".to_owned(),
        },
    );
    reconcile(&platform.admin.dns).await.unwrap();
    provider.remove_by_name("survie.play.kuroctl.test");

    let report = reconcile(&platform.admin.dns).await.unwrap();
    assert_eq!(report.created, 1);
    let published = provider.published();
    assert!(published.contains(&rec(
        "CNAME",
        "survie.play.kuroctl.test",
        "node-1.nodes.play.kuroctl.test"
    )));
    assert!(published.contains(&rec("A", "www.kuroctl.test", "198.51.100.1")));
}

#[tokio::test]
async fn a_provider_outage_is_reported_and_retried() {
    let (platform, _daemon, _id, _) = setup().await;
    *platform.admin.dns_provider.down.lock().unwrap() = true;
    let report = reconcile(&platform.admin.dns).await.unwrap();
    assert_eq!(report.failed, 3);
    let settings = platform.call(Method::GET, "/settings/dns", None).await;
    let records = settings.json["records"].as_array().unwrap();
    assert!(
        records
            .iter()
            .all(|r| r["published"] == false && r["error"].is_string())
    );

    *platform.admin.dns_provider.down.lock().unwrap() = false;
    let report = reconcile(&platform.admin.dns).await.unwrap();
    assert_eq!(report.created, 3);
    let settings = platform.call(Method::GET, "/settings/dns", None).await;
    assert!(
        settings.json["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["published"] == true && r["error"].is_null())
    );
}

#[tokio::test]
async fn removing_the_zone_deletes_what_kuro_created() {
    let (platform, _daemon, _id, _) = setup().await;
    reconcile(&platform.admin.dns).await.unwrap();
    let removed = platform.call(Method::DELETE, "/settings/dns", None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(platform.admin.dns_provider.published().is_empty());
    let settings = platform.call(Method::GET, "/settings/dns", None).await;
    assert_eq!(settings.json["configured"], false);
}

#[tokio::test]
async fn the_zone_needs_its_own_permission() {
    let platform = Platform::start().await;
    let account = platform
        .admin
        .account(
            "settings@example.com",
            &[
                Permission::SystemSettingsRead,
                Permission::SystemSettingsUpdate,
            ],
        )
        .await;
    let token = platform.admin.login(&account).await.access_token;
    let response = platform
        .admin
        .call(Method::GET, "/settings/dns", Some(&token), None)
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json["params"]["permission"], "system.dns.manage");
}
