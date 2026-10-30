//! DNS des serveurs et des nœuds (ADR-0026) : zone configurée avec un jeton vérifié, noms des
//! nœuds et des serveurs, SRV déclaré par le prefab, renommage, réparation des écarts, panne du
//! fournisseur, permissions. Sites web (ADR-0038) : domaine des sites, identifiant et CNAME d'un
//! serveur dont le prefab a un port web.

use axum::http::{Method, StatusCode};
use kuro_api::dns::application::reconcile;
use kuro_api::iam::domain::Permission;
use serde_json::{Value, json};
use uuid::Uuid;

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

/// Serveur WordPress (prefab à port web, sans allocation). Sans adresse, l'API refuse avant tout
/// déploiement : le nœud ne reçoit rien.
async fn create_wordpress(platform: &Platform, node: Uuid) -> crate::support::http::TestResponse {
    platform
        .call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Blog",
                "node_id": node,
                "prefab_id": "wordpress",
                "memory_mb": 1024,
                "cpu_percent": 100,
                "disk_mb": 2048,
                "settings": {},
            })),
        )
        .await
}

#[tokio::test]
async fn a_web_prefab_needs_the_sites_domain_then_gets_its_address_and_cname() {
    let (platform, mut daemon, _, _) = setup().await;
    let node = daemon.node.id.as_uuid();

    // Zone sans domaine des sites : un prefab web n'est pas déployable.
    let refused = create_wordpress(&platform, node).await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(refused.json["code"], "sites_domain_not_configured");

    let zone = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "play.kuroctl.test", "sites_domain": "web.kuroctl.test" }),
    )
    .await;
    assert_eq!(zone.status, StatusCode::OK, "{}", zone.json);
    assert_eq!(zone.json["sites_domain"], "web.kuroctl.test");
    let public = platform.call(Method::GET, "/dns", None).await;
    assert_eq!(public.json["sites_domain"], "web.kuroctl.test");
    let event = platform.admin.last_audit("dns.zone_updated").await;
    assert_eq!(event.details["sites_domain"], "web.kuroctl.test");

    let (response, command) = tokio::join!(create_wordpress(&platform, node), daemon.accept());
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    let web_id = response.json["web_id"].as_str().unwrap().to_owned();
    assert_eq!(web_id.len(), 10);
    assert!(
        web_id
            .bytes()
            .all(|b| b"abcdefghijklmnopqrstuvwxyz234567".contains(&b)),
        "{web_id}"
    );
    assert!(
        response.json["allocations"].as_array().unwrap().is_empty(),
        "a web port has no allocation"
    );
    let url = format!("https://{web_id}.web.kuroctl.test");
    // Le port web n'est pas publié ; l'adresse du site entre dans la configuration de WordPress.
    let web = command["payload"]["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "web")
        .unwrap();
    assert_eq!(web["ports"][0]["container_port"], 80);
    assert!(web["ports"][0].get("published").is_none_or(Value::is_null));
    assert!(
        web["env"]["WORDPRESS_CONFIG_EXTRA"]
            .as_str()
            .unwrap()
            .contains(&format!("define('WP_HOME', '{url}');")),
        "{}",
        web["env"]["WORDPRESS_CONFIG_EXTRA"]
    );
    let created = platform.admin.last_audit("server.created").await;
    assert_eq!(created.details["web_id"], web_id);
    // La route du site part au nœud : nom, service et port du conteneur (ADR-0038).
    assert_eq!(
        command["payload"]["web"],
        json!([{ "host": format!("{web_id}.web.kuroctl.test"), "service": "web", "container_port": 80 }])
    );

    // Le nœud rapporte l'état du proxy et du certificat avec l'état du serveur.
    let server_id = response.json["id"].as_str().unwrap().to_owned();
    daemon
        .event(
            "server.state",
            json!({
                "server_id": server_id,
                "state": "offline",
                "web": { "proxy": "unavailable", "certificate": "pending", "message": "port 443 already in use" }
            }),
        )
        .await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let web_status = loop {
        let server = platform
            .call(Method::GET, &format!("/servers/{server_id}"), None)
            .await;
        if !server.json["web_status"].is_null() {
            break server.json["web_status"].clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "web status never recorded"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(
        web_status,
        json!({ "proxy": "unavailable", "certificate": "pending", "message": "port 443 already in use" })
    );

    // Le CNAME du site vise le nœud, comme celui du serveur.
    reconcile(&platform.admin.dns).await.unwrap();
    let published = platform.admin.dns_provider.published();
    assert!(
        published.contains(&rec(
            "CNAME",
            &format!("{web_id}.web.kuroctl.test"),
            "node-1.nodes.play.kuroctl.test"
        )),
        "{published:?}"
    );
    assert!(published.contains(&rec(
        "CNAME",
        "blog.play.kuroctl.test",
        "node-1.nodes.play.kuroctl.test"
    )));

    // Domaine des sites retiré : le CNAME du site disparaît, l'identifiant reste.
    let without = configure(
        &platform,
        json!({ "provider": "cloudflare", "base_domain": "play.kuroctl.test", "sites_domain": "" }),
    )
    .await;
    assert_eq!(without.status, StatusCode::OK, "{}", without.json);
    assert!(without.json["sites_domain"].is_null());
    reconcile(&platform.admin.dns).await.unwrap();
    assert!(
        !platform
            .admin
            .dns_provider
            .published()
            .iter()
            .any(|(_, name, _)| name.ends_with(".web.kuroctl.test"))
    );
}

#[tokio::test]
async fn the_sites_domain_stays_in_the_zone_apart_from_games_nodes_and_reserved_names() {
    let platform = Platform::start().await;
    let base =
        json!({ "provider": "cloudflare", "base_domain": "play.kuroctl.test", "token": TOKEN });
    for sites in [
        "kuroctl.test",            // la racine de la zone
        "app.kuroctl.test",        // nom réservé
        "play.kuroctl.test",       // le domaine des jeux
        "web.play.kuroctl.test",   // sous le domaine des jeux : collision avec un serveur
        "nodes.play.kuroctl.test", // le sous-domaine des nœuds
        "web.other.test",          // une autre zone du fournisseur
        "not a domain",
    ] {
        let mut body = base.clone();
        body["sites_domain"] = json!(sites);
        let refused = configure(&platform, body).await;
        assert_eq!(
            refused.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{sites}: {}",
            refused.json
        );
        assert_eq!(
            refused.json["fields"][0]["field"], "sites_domain",
            "{sites}"
        );
        assert_eq!(
            refused.json["fields"][0]["code"], "invalid_sites_domain",
            "{sites}"
        );
    }
    let settings = platform.call(Method::GET, "/settings/dns", None).await;
    assert_eq!(settings.json["configured"], false, "nothing saved");

    let mut body = base.clone();
    body["sites_domain"] = json!(" Sites.Kuroctl.test. ");
    let saved = configure(&platform, body).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(saved.json["sites_domain"], "sites.kuroctl.test");
}
