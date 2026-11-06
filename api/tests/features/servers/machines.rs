//! Nœuds à la demande (ADR-0065) : réglages et identifiants jamais relus, location qui enrôle un
//! nœud et crée le serveur attendu dès qu'il est en ligne, refus du fournisseur, réponse perdue
//! rattachée, orphelines (seulement celles de l'instance, simulation d'abord), sommeil et réveil,
//! retour, permissions. Tâches de fond (ADR-0062) : réglage de simulation, lancement, rapport.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use kuro_api::machines::application::sweep;
use kuro_api::machines::domain::{LABEL, SCOPE_LABEL};
use kuro_daemon::config::Config as DaemonConfig;
use serde_json::{Value, json};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::support::link::{TestNode, say_hello};
use crate::support::platform::{FakeDaemon, Platform};

/// Région `eu-west` louée chez le fournisseur simulé (`hetzner`, `fsn1`) ; son identifiant.
async fn configured(platform: &Platform) -> String {
    let region = platform
        .call(
            Method::POST,
            "/regions",
            Some(json!({ "slug": "eu-west", "name": "Europe Ouest", "latitude": 48.85, "longitude": 2.35 })),
        )
        .await;
    assert_eq!(region.status, StatusCode::CREATED, "{}", region.json);
    let region_id = region.json["id"].as_str().unwrap().to_owned();
    let provider = platform
        .call(
            Method::PUT,
            "/settings/machines/providers/hetzner",
            Some(json!({ "token": "secret-token" })),
        )
        .await;
    assert_eq!(provider.status, StatusCode::OK, "{}", provider.json);
    let settings = platform
        .call(
            Method::PUT,
            "/settings/machines",
            Some(json!({
                "api_url": "https://panel.example.com:8443/",
                "port_range": "25565-25570",
                "regions": [{ "region_id": region_id, "provider": "hetzner", "location": "fsn1" }],
            })),
        )
        .await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.json);
    region_id
}

async fn rent(platform: &Platform, body: Value) -> crate::support::http::TestResponse {
    platform.call(Method::POST, "/machines", Some(body)).await
}

/// Jeton d'enrôlement écrit dans le cloud-init de la dernière machine créée.
fn enrollment_token(platform: &Platform) -> String {
    let spec = platform.admin.machine_provider.created().pop().unwrap();
    let line = spec
        .user_data
        .lines()
        .find(|line| line.trim_start().starts_with("content:"))
        .unwrap();
    serde_json::from_str(line.trim_start().trim_start_matches("content:").trim()).unwrap()
}

/// Le nœud de la machine s'enrôle avec le jeton du cloud-init et se connecte.
async fn machine_comes_online(platform: &Platform, machine: &Value) -> FakeDaemon {
    let name = machine["node_name"].as_str().unwrap();
    let token = enrollment_token(platform);
    let enrolled = platform
        .api
        .enroll_from(name, &token, &platform.api.pki.fingerprint)
        .await
        .unwrap();
    let node = TestNode {
        id: Uuid::parse_str(machine["node_id"].as_str().unwrap())
            .unwrap()
            .into(),
        config: DaemonConfig::load(&enrolled.config_path).unwrap(),
    };
    let mut ws = platform.api.connect(&node).await;
    say_hello(&mut ws).await;
    platform
        .api
        .wait_for_status(node.id.as_uuid(), "online")
        .await;
    FakeDaemon { node, ws }
}

async fn machine(platform: &Platform, id: &str) -> Value {
    let page = platform.call(Method::GET, "/machines", None).await;
    page.json["machines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}

#[tokio::test]
async fn without_provider_the_page_asks_to_configure_one_and_tokens_are_never_returned() {
    let platform = Platform::start().await;
    let page = platform.call(Method::GET, "/machines", None).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.json);
    assert_eq!(page.json["configured"], false);
    assert_eq!(page.json["sweep"]["name"], "machines.sweep");
    assert_eq!(page.json["sweep"]["dry_run"], true);

    let region_id = configured(&platform).await;
    let settings = platform.call(Method::GET, "/settings/machines", None).await;
    assert!(!settings.body.contains("secret-token"));
    assert_eq!(settings.json["providers"][0]["name"], "hetzner");
    assert_eq!(settings.json["providers"][0]["configured"], true);
    assert_eq!(settings.json["api_url"], "https://panel.example.com:8443");
    let region = settings.json["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["slug"] == "eu-west")
        .unwrap();
    assert_eq!(region["region_id"], region_id.as_str());
    assert_eq!(region["location"], "fsn1");
    assert_eq!(
        platform
            .admin
            .last_audit("machines.provider_updated")
            .await
            .target_id
            .as_deref(),
        Some("hetzner")
    );

    let bad = platform
        .call(
            Method::PUT,
            "/settings/machines",
            Some(json!({ "api_url": "ftp://x", "port_range": "25565-25570" })),
        )
        .await;
    assert_eq!(bad.json["code"], "invalid_api_url", "{}", bad.json);
    let in_use = platform
        .call(Method::DELETE, "/settings/machines/providers/hetzner", None)
        .await;
    assert_eq!(in_use.json["code"], "machine_provider_in_use");
    let page = platform.call(Method::GET, "/machines", None).await;
    assert_eq!(page.json["configured"], true);
}

#[tokio::test]
async fn a_rented_machine_enrolls_and_receives_the_waiting_server() {
    let platform = Platform::start().await;
    let region_id = configured(&platform).await;
    let offers = platform
        .call(
            Method::GET,
            &format!("/machines/offers?region_id={region_id}"),
            None,
        )
        .await;
    assert_eq!(offers.json[0]["name"], "small", "{}", offers.json);

    let rented = rent(
        &platform,
        json!({
            "region_id": region_id,
            "offer": "small",
            "server": {
                "name": "Survie",
                "prefab_id": "minecraft-java",
                "memory_mb": 2048,
                "cpu_percent": 200,
                "disk_mb": 10240,
                "settings": {},
            },
        }),
    )
    .await;
    assert_eq!(rented.status, StatusCode::CREATED, "{}", rented.json);
    let machine = rented.json;
    assert_eq!(machine["state"], "ready");
    assert_eq!(machine["node_status"], "pending");
    // Le serveur existe tout de suite, en attente de sa machine, avec son adresse.
    let server_id = machine["server_id"].as_str().unwrap().to_owned();
    assert_eq!(machine["server_state"], "waiting_node");
    let waiting = platform
        .call(Method::GET, &format!("/servers/{server_id}"), None)
        .await;
    assert_eq!(waiting.json["state"], "waiting_node", "{}", waiting.json);
    assert!(
        waiting.body.contains(machine["ipv4"].as_str().unwrap()),
        "{}",
        waiting.json
    );
    assert!(!waiting.body.contains("0.0.0.0"), "{}", waiting.json);
    let spec = platform.admin.machine_provider.created().pop().unwrap();
    assert_eq!(spec.offer, "small");
    assert_eq!(spec.location, "fsn1");
    assert_eq!(
        spec.labels.get(LABEL),
        machine["id"].as_str().map(str::to_owned).as_ref()
    );
    assert!(spec.labels.contains_key(SCOPE_LABEL));
    assert!(spec.user_data.contains("https://panel.example.com:8443"));

    // Le nœud loué par l'admin n'est pas réservé ; il porte sa machine.
    let node_id = machine["node_id"].as_str().unwrap();
    let node = platform
        .call(Method::GET, &format!("/nodes/{node_id}"), None)
        .await;
    assert_eq!(node.json["reserved"], false);
    assert_eq!(node.json["machine_id"], machine["id"]);
    // L'adresse publique de la VM devient celle du nœud.
    assert_eq!(node.json["public_ipv4"], machine["ipv4"]);

    // Le nœud se connecte : son serveur est déployé aussitôt, comme une création.
    let mut daemon = machine_comes_online(&platform, &machine).await;
    let command = daemon.accept().await;
    assert_eq!(command["type"], "server.deploy", "{command}");
    assert_eq!(
        command["payload"]["server_id"],
        server_id.as_str(),
        "{command}"
    );
    let deploying = platform
        .call(Method::GET, &format!("/servers/{server_id}"), None)
        .await;
    assert_eq!(deploying.json["state"], "deploying", "{}", deploying.json);

    let report = sweep(&platform.admin.machines, OffsetDateTime::now_utc(), true)
        .await
        .unwrap();
    assert_eq!(report.nodes_online, 1);
    let id = machine["id"].as_str().unwrap();
    assert!(self::machine(&platform, id).await["node_online_at"].is_string());
    let servers = platform.call(Method::GET, "/servers", None).await;
    assert!(servers.body.contains(node_id), "{}", servers.json);

    // Le nœud porte un serveur : le rendre est refusé.
    let refused = platform
        .call(Method::DELETE, &format!("/machines/{id}"), None)
        .await;
    assert_eq!(refused.json["code"], "node_has_servers");
}

#[tokio::test]
async fn a_refused_creation_leaves_nothing_and_a_lost_answer_is_adopted() {
    let platform = Platform::start().await;
    let region_id = configured(&platform).await;

    *platform.admin.machine_provider.fail.lock().unwrap() = Some("quota".to_owned());
    let refused = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "small" }),
    )
    .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(
        refused.json["code"], "machine_provider_refused",
        "{}",
        refused.json
    );
    let page = platform.call(Method::GET, "/machines", None).await;
    assert_eq!(page.json["machines"].as_array().unwrap().len(), 0);
    let nodes = platform.call(Method::GET, "/nodes", None).await;
    assert_eq!(nodes.json.as_array().unwrap().len(), 0, "{}", nodes.json);
    assert_eq!(
        platform.admin.last_audit("machine.abandoned").await.outcome,
        "failure"
    );

    let unknown = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "huge" }),
    )
    .await;
    assert_eq!(
        unknown.json["code"], "unknown_machine_offer",
        "{}",
        unknown.json
    );

    *platform.admin.machine_provider.lose_answer.lock().unwrap() = true;
    let lost = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "small" }),
    )
    .await;
    assert_eq!(lost.status, StatusCode::CREATED, "{}", lost.json);
    assert_eq!(lost.json["state"], "creating");
    let deleted = platform
        .call(
            Method::DELETE,
            &format!("/machines/{}", lost.json["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(deleted.json["code"], "machine_creating");

    let report = sweep(&platform.admin.machines, OffsetDateTime::now_utc(), true)
        .await
        .unwrap();
    assert_eq!(report.adopted, 1);
    let adopted = machine(&platform, lost.json["id"].as_str().unwrap()).await;
    assert_eq!(adopted["state"], "ready");
    assert!(adopted["reference"].is_string());
}

#[tokio::test]
async fn orphans_of_this_instance_only_are_deleted_and_simulation_comes_first() {
    let platform = Platform::start().await;
    configured(&platform).await;
    let tag: String = sqlx::query_scalar("SELECT instance_tag FROM machine_settings")
        .fetch_one(&platform.api.pool)
        .await
        .unwrap();
    let provider = &platform.admin.machine_provider;
    let old = OffsetDateTime::now_utc() - time::Duration::hours(2);
    let young = OffsetDateTime::now_utc() - time::Duration::minutes(5);
    let ghost = Uuid::now_v7().to_string();
    let mine = provider.add_vm(&[(LABEL, &ghost), (SCOPE_LABEL, &tag)], LABEL, old);
    let starting = provider.add_vm(&[(LABEL, "other"), (SCOPE_LABEL, &tag)], LABEL, young);
    let foreign = provider.add_vm(&[(LABEL, "x"), (SCOPE_LABEL, "another-kuro")], LABEL, old);

    let now = OffsetDateTime::now_utc();
    let simulated = sweep(&platform.admin.machines, now, true).await.unwrap();
    assert_eq!(simulated.orphans, 1);
    assert!(provider.deleted().is_empty());

    let report = sweep(&platform.admin.machines, now, false).await.unwrap();
    assert_eq!(report.orphans, 1);
    assert_eq!(provider.deleted(), vec![mine]);
    let left: Vec<String> = provider.vms().into_iter().map(|vm| vm.reference).collect();
    assert!(left.contains(&starting) && left.contains(&foreign));
    let event = platform.admin.last_audit("machine.orphan_deleted").await;
    assert_eq!(event.target_id.as_deref(), Some(ghost.as_str()));
}

#[tokio::test]
async fn a_machine_sleeps_on_a_snapshot_wakes_up_and_is_released() {
    let platform = Platform::start().await;
    let region_id = configured(&platform).await;
    let rented = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "small" }),
    )
    .await;
    let machine = rented.json;
    let id = machine["id"].as_str().unwrap().to_owned();
    let mut daemon = machine_comes_online(&platform, &machine).await;
    let reference = machine["reference"].as_str().unwrap().to_owned();

    let asleep = platform
        .call(Method::POST, &format!("/machines/{id}/sleep"), None)
        .await;
    assert_eq!(asleep.status, StatusCode::OK, "{}", asleep.json);
    assert_eq!(asleep.json["state"], "asleep");
    assert_eq!(asleep.json["has_snapshot"], true);
    let provider = &platform.admin.machine_provider;
    assert!(provider.deleted().contains(&reference));
    assert_eq!(provider.snapshots().len(), 1);
    // Le nœud reste dans Kuro, hors ligne une fois sa VM rendue.
    drop(daemon.ws);

    let awake = platform
        .call(Method::POST, &format!("/machines/{id}/wake"), None)
        .await;
    assert_eq!(awake.status, StatusCode::OK, "{}", awake.json);
    assert_eq!(awake.json["state"], "waking");
    let spec = provider.created().pop().unwrap();
    assert_eq!(
        spec.snapshot.as_deref(),
        provider.snapshots().first().map(String::as_str)
    );

    // Le nœud se reconnecte avec son certificat : réveil fini, snapshot supprimé.
    platform
        .api
        .wait_for_status(daemon.node.id.as_uuid(), "offline")
        .await;
    let mut ws = platform.api.connect(&daemon.node).await;
    say_hello(&mut ws).await;
    platform
        .api
        .wait_for_status(daemon.node.id.as_uuid(), "online")
        .await;
    daemon.ws = ws;
    let report = sweep(&platform.admin.machines, OffsetDateTime::now_utc(), true)
        .await
        .unwrap();
    assert_eq!(report.awake, 1);
    assert!(provider.snapshots().is_empty());
    assert_eq!(self::machine(&platform, &id).await["state"], "ready");

    let released = platform
        .call(Method::DELETE, &format!("/machines/{id}"), None)
        .await;
    assert_eq!(released.status, StatusCode::NO_CONTENT, "{}", released.json);
    assert!(provider.vms().is_empty());
    assert_eq!(self::machine(&platform, &id).await, Value::Null);
    let node = platform
        .call(
            Method::GET,
            &format!("/nodes/{}", daemon.node.id.as_uuid()),
            None,
        )
        .await;
    assert_eq!(node.status, StatusCode::NOT_FOUND);
    assert_eq!(
        platform.admin.last_audit("machine.released").await.outcome,
        "success"
    );
}

#[tokio::test]
async fn renting_and_settings_follow_permissions() {
    let platform = Platform::start().await;
    let region_id = configured(&platform).await;
    let account = platform
        .admin
        .account("reader@example.com", &[Permission::NodesRead])
        .await;
    let reader = platform.admin.login(&account).await.access_token;
    let call = |method, path: &str, body| {
        let reader = reader.clone();
        let path = path.to_owned();
        let admin = &platform.admin;
        async move { admin.call(method, &path, Some(&reader), body).await }
    };
    assert_eq!(
        call(Method::GET, "/machines", None).await.status,
        StatusCode::OK
    );
    let rent = call(
        Method::POST,
        "/machines",
        Some(json!({ "region_id": region_id, "offer": "small" })),
    )
    .await;
    assert_eq!(rent.status, StatusCode::FORBIDDEN);
    let settings = call(Method::GET, "/settings/machines", None).await;
    assert_eq!(settings.status, StatusCode::FORBIDDEN);
    let tasks = call(Method::GET, "/settings/tasks", None).await;
    assert_eq!(tasks.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_task_runs_once_with_its_simulation_setting_and_records_its_report() {
    let platform = Platform::start().await;
    let tasks = platform.call(Method::GET, "/settings/tasks", None).await;
    assert_eq!(tasks.status, StatusCode::OK, "{}", tasks.json);
    assert_eq!(tasks.json[0]["name"], "machines.sweep");
    assert_eq!(tasks.json[0]["dry_run"], true);
    assert_eq!(tasks.json[0]["period_secs"], 60);

    let updated = platform
        .call(
            Method::PUT,
            "/settings/tasks/machines.sweep",
            Some(json!({ "dry_run": false })),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.json);
    assert_eq!(updated.json["dry_run"], false);
    assert_eq!(
        platform
            .admin
            .last_audit("task.updated")
            .await
            .target_id
            .as_deref(),
        Some("machines.sweep")
    );
    let unknown = platform
        .call(Method::POST, "/settings/tasks/nope/run", None)
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let triggered = platform
        .call(Method::POST, "/settings/tasks/machines.sweep/run", None)
        .await;
    assert_eq!(triggered.status, StatusCode::ACCEPTED);

    let shutdown = CancellationToken::new();
    let status = platform
        .admin
        .tasks
        .pass("machines.sweep", &shutdown)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.outcome.as_deref(), Some("ok"));
    assert_eq!(status.last_dry_run, Some(false));
    assert!(!status.running);
    assert_eq!(status.report.unwrap()["dry_run"], false);

    // Une autre instance qui tient le verrou : ce passage-ci est sauté.
    let mut held = platform.api.pool.acquire().await.unwrap();
    sqlx::query("SELECT pg_advisory_lock(hashtext('task:machines.sweep')::bigint)")
        .execute(&mut *held)
        .await
        .unwrap();
    let skipped = platform
        .admin
        .tasks
        .pass("machines.sweep", &shutdown)
        .await
        .unwrap();
    assert!(skipped.is_none());
}

#[tokio::test]
async fn a_failing_or_panicking_task_is_recorded_and_an_extension_task_is_prefixed() {
    use kuro_api::extension::{Extension, ExtensionFactory};
    use kuro_api::tasks::{Task, TaskDef, TaskError, Tasks};

    let platform = Platform::start().await;
    let def = |name| TaskDef {
        name,
        period: std::time::Duration::from_secs(3600),
        dry_run_by_default: false,
    };
    let tasks = Tasks::new(
        platform.api.pool.clone(),
        vec![
            Task::new("test", def("fails"), None, |_| {
                Box::pin(async { Err(TaskError("provider down".to_owned())) })
            }),
            Task::new("test", def("panics"), None, |_| {
                Box::pin(async { panic!("boom") })
            }),
        ],
    );
    tasks.attach(kuro_api::state::AppHandle::new(&platform.admin.app));
    let shutdown = CancellationToken::new();
    let failed = tasks.pass("test.fails", &shutdown).await.unwrap().unwrap();
    assert_eq!(failed.outcome.as_deref(), Some("error"));
    assert_eq!(failed.error.as_deref(), Some("provider down"));
    let panicked = tasks.pass("test.panics", &shutdown).await.unwrap().unwrap();
    assert_eq!(panicked.error.as_deref(), Some("task panicked"));
    assert!(!panicked.running);

    let factory = ExtensionFactory::new("tasks_demo", move |_| {
        Ok(Extension::default().task(def("tick"), |context| {
            Box::pin(async move { Ok(json!({ "dry_run": context.dry_run })) })
        }))
    });
    let mounted = kuro_api::extension::prepare(
        &platform.api.pool,
        &std::collections::BTreeMap::new(),
        vec![factory],
    )
    .await
    .unwrap();
    let names: Vec<&str> = mounted[0].tasks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["tasks_demo.tick"]);
}

#[tokio::test]
async fn a_server_never_asks_for_more_cores_than_its_node_or_machine_has() {
    let platform = Platform::start().await;
    // Le faux nœud annonce 4 cœurs : 5 sont refusés avant tout déploiement.
    let daemon = platform.online_node("node-1", "25565-25570").await;
    let refused = platform
        .call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "memory_mb": 2048,
                "cpu_percent": 500,
                "disk_mb": 10240,
            })),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        refused.json
    );
    assert_eq!(
        refused.json["fields"][0]["code"], "cpu_above_node",
        "{}",
        refused.json
    );

    // L'offre `small` (2 vCPU) ne porte pas un serveur de 3 cœurs : rien n'est loué.
    let region_id = configured(&platform).await;
    let too_small = rent(
        &platform,
        json!({
            "region_id": region_id,
            "offer": "small",
            "server": { "name": "Gros", "prefab_id": "minecraft-java", "cpu_percent": 300, "memory_mb": 2048, "disk_mb": 10240 },
        }),
    )
    .await;
    assert_eq!(
        too_small.json["code"], "machine_offer_too_small",
        "{}",
        too_small.json
    );
    assert!(platform.admin.machine_provider.created().is_empty());

    // Sans ressources fixées, le serveur prend la machine entière, réserve du système déduite.
    let whole = rent(
        &platform,
        json!({
            "region_id": region_id,
            "offer": "small",
            "server": { "name": "Entier", "prefab_id": "minecraft-java" },
        }),
    )
    .await;
    assert_eq!(whole.status, StatusCode::CREATED, "{}", whole.json);
    let server = platform
        .call(
            Method::GET,
            &format!("/servers/{}", whole.json["server_id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(server.json["cpu_percent"], 200, "{}", server.json);
    assert_eq!(server.json["memory_mb"], 3072);
    assert_eq!(server.json["disk_mb"], 30 * 1024);

    // Petite machine (1 vCPU, 1 Go, 20 Go) : la réserve suit sa taille, le serveur garde 768 Mo.
    let tiny = rent(
        &platform,
        json!({
            "region_id": region_id,
            "offer": "tiny",
            "server": { "name": "Petit", "prefab_id": "minecraft-java" },
        }),
    )
    .await;
    assert_eq!(tiny.status, StatusCode::CREATED, "{}", tiny.json);
    let small = platform
        .call(
            Method::GET,
            &format!("/servers/{}", tiny.json["server_id"].as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(small.json["memory_mb"], 768, "{}", small.json);
    assert_eq!(small.json["cpu_percent"], 100);
    assert_eq!(small.json["disk_mb"], 10 * 1024);
}

#[tokio::test]
async fn the_server_of_a_lost_machine_fails_with_its_reason_and_can_be_deleted() {
    let platform = Platform::start().await;
    let region_id = configured(&platform).await;
    // Le fournisseur refuse : rien n'est loué, et le serveur préparé disparaît avec la machine.
    *platform.admin.machine_provider.fail.lock().unwrap() = Some("quota".to_owned());
    let refused = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "small",
                "server": { "name": "Refusé", "prefab_id": "minecraft-java" } }),
    )
    .await;
    assert_eq!(
        refused.json["code"], "machine_provider_refused",
        "{}",
        refused.json
    );
    let servers = platform.call(Method::GET, "/servers", None).await;
    assert!(!servers.body.contains("Refusé"), "{}", servers.json);

    // Réponse perdue, puis aucune VM dans le délai : le serveur passe en échec avec la raison.
    *platform.admin.machine_provider.lose_answer.lock().unwrap() = true;
    let lost = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "small",
                "server": { "name": "Perdu", "prefab_id": "minecraft-java" } }),
    )
    .await;
    assert_eq!(lost.status, StatusCode::CREATED, "{}", lost.json);
    let server_id = lost.json["server_id"].as_str().unwrap().to_owned();
    for vm in platform.admin.machine_provider.vms() {
        platform.admin.machine_provider.delete_now(&vm.reference);
    }
    let later = OffsetDateTime::now_utc() + time::Duration::hours(1);
    let report = sweep(&platform.admin.machines, later, false).await.unwrap();
    assert_eq!(report.abandoned, 1);
    let failed = platform
        .call(Method::GET, &format!("/servers/{server_id}"), None)
        .await;
    assert_eq!(failed.json["state"], "failed", "{}", failed.json);
    assert_eq!(
        failed.json["state_reason"], "machine_unavailable",
        "{}",
        failed.json
    );

    // Jamais déployé : supprimé sans attendre de nœud.
    let deleted = platform
        .call(Method::DELETE, &format!("/servers/{server_id}"), None)
        .await;
    assert!(deleted.status.is_success(), "{}", deleted.json);
    let gone = platform
        .call(Method::GET, &format!("/servers/{server_id}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn saving_a_token_creates_a_disabled_region_per_location_and_takes_its_place() {
    let platform = Platform::start().await;
    let saved = platform
        .call(
            Method::PUT,
            "/settings/machines/providers/hetzner",
            Some(json!({ "token": "secret-token" })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.json);
    assert_eq!(
        saved.json["created_regions"],
        json!(["Falkenstein (DE)", "Nuremberg (DE)", "Ashburn, VA (US)"])
    );
    let regions = platform.call(Method::GET, "/regions", None).await;
    let falkenstein = regions
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["slug"] == "falkenstein")
        .unwrap()
        .clone();
    assert_eq!(falkenstein["enabled"], false, "{falkenstein}");
    let hosting = |settings: &Value, slug: &str| {
        settings["regions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["slug"] == slug)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        hosting(&saved.json["settings"], "falkenstein")["location"],
        "fsn1"
    );
    assert_eq!(
        hosting(&saved.json["settings"], "ashburn-va")["location"],
        "ash"
    );

    let locations = platform
        .call(
            Method::GET,
            "/settings/machines/providers/hetzner/locations",
            None,
        )
        .await;
    assert_eq!(locations.json[0]["name"], "fsn1", "{}", locations.json);
    assert_eq!(locations.json[0]["city"], "Falkenstein");

    // Une région à soi reliée à fsn1 est remplacée par celle de l'emplacement au prochain jeton.
    let own = platform
        .call(
            Method::POST,
            "/regions",
            Some(json!({ "slug": "eu-west", "name": "Europe Ouest", "latitude": 48.85, "longitude": 2.35 })),
        )
        .await;
    let own_id = own.json["id"].as_str().unwrap();
    let mapped = platform
        .call(
            Method::PUT,
            "/settings/machines",
            Some(json!({ "port_range": "25565-25600",
                         "regions": [{ "region_id": own_id, "provider": "hetzner", "location": "fsn1" }] })),
        )
        .await;
    assert_eq!(mapped.status, StatusCode::OK, "{}", mapped.json);
    let again = platform
        .call(
            Method::PUT,
            "/settings/machines/providers/hetzner",
            Some(json!({ "token": "secret-token" })),
        )
        .await;
    assert_eq!(again.json["created_regions"], json!([]), "{}", again.json);
    // Le bouton rejoue la même chose sans retaper le jeton : une région supprimée revient.
    let ashburn = regions
        .json
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["slug"] == "ashburn-va")
        .unwrap();
    let removed = platform
        .call(
            Method::DELETE,
            &format!("/regions/{}", ashburn["id"].as_str().unwrap()),
            None,
        )
        .await;
    assert!(removed.status.is_success(), "{}", removed.json);
    let button = platform
        .call(
            Method::POST,
            "/settings/machines/providers/hetzner/regions",
            None,
        )
        .await;
    assert_eq!(
        button.json["created_regions"],
        json!(["Ashburn, VA (US)"]),
        "{}",
        button.json
    );
    assert_eq!(
        hosting(&again.json["settings"], "falkenstein")["location"],
        "fsn1"
    );
    assert!(hosting(&again.json["settings"], "eu-west")["provider"].is_null());
}

#[tokio::test]
async fn an_extension_task_and_its_hooks_receive_the_application_state() {
    use kuro_api::extension::{Extension, ExtensionFactory};
    use kuro_api::tasks::TaskDef;
    use std::sync::{Arc, OnceLock};

    let ready: Arc<OnceLock<kuro_api::state::AppHandle>> = Arc::default();
    let seen = ready.clone();
    let factory = ExtensionFactory::new("app_demo", move |_| {
        let seen = seen.clone();
        Ok(Extension::default()
            .on_ready(move |app| {
                let _ = seen.set(app);
            })
            .task(
                TaskDef {
                    name: "count",
                    period: std::time::Duration::from_secs(3600),
                    dry_run_by_default: false,
                },
                |context| {
                    Box::pin(async move {
                        // Cas d'usage de Kuro depuis une tâche : ici, les machines et les tâches.
                        let machines =
                            kuro_api::machines::application::list(&context.app.pool, None)
                                .await
                                .map_err(|e| kuro_api::tasks::TaskError(e.to_string()))?;
                        let tasks = context
                            .app
                            .tasks
                            .list()
                            .await
                            .map_err(|e| kuro_api::tasks::TaskError(e.to_string()))?;
                        Ok(json!({ "machines": machines.len(), "tasks": tasks.len() }))
                    })
                },
            ))
    });
    let platform = Platform::start_with_extensions(vec![factory]).await;

    // `on_ready` a reçu l'état avant la première requête.
    let app = ready
        .get()
        .and_then(|handle| handle.get())
        .expect("on_ready called");
    assert!(app.tasks.status("app_demo.count").await.is_ok());

    let status = platform
        .admin
        .tasks
        .pass("app_demo.count", &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.outcome.as_deref(), Some("ok"), "{:?}", status.error);
    assert_eq!(status.report.unwrap(), json!({ "machines": 0, "tasks": 2 }));
}

#[tokio::test]
async fn node_capacity_counts_every_server_including_those_waiting_for_their_node() {
    use kuro_api::nodes::application::inventory::capacity;
    use kuro_api::nodes::infra::RegionFilter;

    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    platform.deploy_minecraft(&mut daemon, json!({})).await;
    let region_id = configured(&platform).await;
    let rented = rent(
        &platform,
        json!({ "region_id": region_id, "offer": "small",
                "server": { "name": "Attente", "prefab_id": "minecraft-java", "cpu_percent": 150, "memory_mb": 1024 } }),
    )
    .await;
    assert_eq!(rented.status, StatusCode::CREATED, "{}", rented.json);

    let all = capacity(&platform.api.pool, RegionFilter::All)
        .await
        .unwrap();
    let online = all
        .iter()
        .find(|n| n.node_id == daemon.node.id.as_uuid())
        .unwrap();
    assert_eq!(online.status, "online");
    assert_eq!(online.cpu_cores, Some(4));
    assert_eq!(online.memory_bytes, Some(8_589_934_592));
    // `deploy_minecraft` : 2048 Mo, 200 %.
    assert_eq!(
        (
            online.allocated_cpu_percent,
            online.allocated_memory_mb,
            online.servers
        ),
        (200, 2048, 1)
    );

    let region = Uuid::parse_str(&region_id).unwrap();
    let in_region = capacity(&platform.api.pool, RegionFilter::Region(region))
        .await
        .unwrap();
    assert_eq!(in_region.len(), 1);
    let machine = &in_region[0];
    assert_eq!(machine.status, "pending");
    assert_eq!(machine.cpu_cores, None);
    assert_eq!(
        (
            machine.allocated_cpu_percent,
            machine.allocated_memory_mb,
            machine.servers
        ),
        (150, 1024, 1)
    );
}
