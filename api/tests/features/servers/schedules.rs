//! Tâches planifiées (ADR-0022) : définition et validation, anti-escalade, moteur (ordre des
//! étapes, attente de la sauvegarde, sauts, échecs, non-chevauchement), lancement manuel, audit.

use axum::http::{Method, StatusCode};
use kuro_api::iam::domain::Permission;
use kuro_api::servers::application::scheduler;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::support::platform::{FakeDaemon, Platform};

struct Setup {
    platform: Platform,
    daemon: FakeDaemon,
    id: String,
}

/// Serveur Minecraft déployé, dans l'état demandé (`running`, `offline`).
async fn server(state: &str) -> Setup {
    let platform = Platform::start().await;
    let mut daemon = platform.online_node("node-1", "25565").await;
    let (server, _) = platform.deploy_minecraft(&mut daemon, json!({})).await;
    let id = server["id"].as_str().unwrap().to_owned();
    daemon.report(&id, state).await;
    platform.wait_for_state(&id, state).await;
    Setup {
        platform,
        daemon,
        id,
    }
}

fn definition(steps: Value) -> Value {
    json!({
        "name": "Nuit",
        "cron": "0 4 * * *",
        "timezone": "Europe/Paris",
        "steps": steps,
    })
}

async fn create(setup: &Setup, body: Value) -> Value {
    let response = setup
        .platform
        .call(
            Method::POST,
            &format!("/servers/{}/schedules", setup.id),
            Some(body),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json
}

fn next_run(schedule: &Value) -> OffsetDateTime {
    OffsetDateTime::parse(schedule["next_run_at"].as_str().unwrap(), &Rfc3339).unwrap()
}

/// Passe du moteur à `now`, exécutions attendues jusqu'au bout.
async fn run_due(setup: &Setup, now: OffsetDateTime) {
    for handle in scheduler::run_due(&setup.platform.admin.servers, now)
        .await
        .unwrap()
    {
        handle.await.unwrap();
    }
}

async fn runs(setup: &Setup, schedule: &Value) -> Vec<Value> {
    let response = setup
        .platform
        .call(
            Method::GET,
            &format!(
                "/servers/{}/schedules/{}/runs",
                setup.id,
                schedule["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    response.json.as_array().unwrap().clone()
}

#[tokio::test]
async fn a_schedule_gets_its_next_run_in_its_timezone_and_is_audited_without_its_commands() {
    let setup = server("offline").await;

    let schedule = create(
        &setup,
        definition(json!([
            { "action": "command", "value": "say secret-token" },
            { "action": "power", "value": "restart", "delay_secs": 30 },
            { "action": "backup" },
        ])),
    )
    .await;

    assert_eq!(schedule["enabled"], true);
    assert_eq!(schedule["running"], false);
    assert_eq!(schedule["steps"][1]["value"], "restart");
    assert_eq!(schedule["steps"][1]["delay_secs"], 30);
    assert_eq!(schedule["steps"][2]["value"], Value::Null);
    // 04:00 à Paris tombe à 02:00 ou 03:00 UTC selon la saison.
    let next = next_run(&schedule);
    assert!(next > OffsetDateTime::now_utc());
    assert!([2, 3].contains(&next.hour()), "{next}");

    let event = setup
        .platform
        .admin
        .last_audit("server.schedule_created")
        .await;
    assert_eq!(
        event.details["schedule"]["steps"],
        json!(["command", "power", "backup"])
    );
    assert!(!event.details.to_string().contains("secret-token"));
}

#[tokio::test]
async fn invalid_definitions_are_refused_under_their_field() {
    let setup = server("offline").await;
    let path = format!("/servers/{}/schedules", setup.id);
    let cases = [
        (json!({ "cron": "0 0 4 * * *" }), "cron", "invalid_cron"),
        (json!({ "cron": "0 0 31 2 *" }), "cron", "invalid_cron"),
        (
            json!({ "timezone": "Mars/Olympus" }),
            "timezone",
            "invalid_timezone",
        ),
        (json!({ "name": "  " }), "name", "invalid_schedule_name"),
        (json!({ "steps": [] }), "steps", "invalid_schedule_step"),
        (
            json!({ "steps": [{ "action": "power", "value": "explode" }] }),
            "steps.0.value",
            "invalid_schedule_step",
        ),
        (
            json!({ "steps": [{ "action": "command", "value": "say hi", "delay_secs": 4000 }] }),
            "steps.0.delay_secs",
            "invalid_schedule_step",
        ),
    ];
    for (overrides, field, code) in cases {
        let mut body = definition(json!([{ "action": "command", "value": "save-all" }]));
        body.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        let response = setup.platform.call(Method::POST, &path, Some(body)).await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{field}: {}",
            response.json
        );
        assert_eq!(
            response.json["fields"][0]["field"], field,
            "{}",
            response.json
        );
        assert_eq!(
            response.json["fields"][0]["code"], code,
            "{}",
            response.json
        );
    }
}

#[tokio::test]
async fn the_preview_lists_the_next_runs() {
    let setup = server("offline").await;
    let response = setup
        .platform
        .call(
            Method::POST,
            &format!("/servers/{}/schedules/preview", setup.id),
            Some(json!({ "cron": "0 */6 * * *", "timezone": "UTC" })),
        )
        .await;

    assert_eq!(response.status, StatusCode::OK, "{}", response.json);
    let runs = response.json["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 3);
    for run in runs {
        let at = OffsetDateTime::parse(run.as_str().unwrap(), &Rfc3339).unwrap();
        assert_eq!(at.hour() % 6, 0);
        assert_eq!(at.minute(), 0);
    }
}

#[tokio::test]
async fn a_schedule_cannot_do_what_its_author_cannot() {
    let setup = server("offline").await;
    let admin = &setup.platform.admin;
    let account = admin
        .account(
            "planner@example.com",
            &[
                Permission::ServersSchedulesRead,
                Permission::ServersSchedulesManage,
                Permission::ServersPower,
            ],
        )
        .await;
    let token = admin.login(&account).await.access_token;
    let path = format!("/servers/{}/schedules", setup.id);

    let refused = admin
        .call(
            Method::POST,
            &path,
            Some(&token),
            Some(definition(
                json!([{ "action": "command", "value": "op mallory" }]),
            )),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.json);
    assert_eq!(
        refused.json["params"]["permission"],
        "servers.console.write"
    );
    let denial = admin.last_audit("access.denied").await;
    assert_eq!(denial.details["permission"], "servers.console.write");

    let allowed = admin
        .call(
            Method::POST,
            &path,
            Some(&token),
            Some(definition(
                json!([{ "action": "power", "value": "restart" }]),
            )),
        )
        .await;
    assert_eq!(allowed.status, StatusCode::CREATED, "{}", allowed.json);

    // Une tâche d'un autre, avec une commande, ne peut être ni gardée ni lancée.
    let foreign = create(
        &setup,
        definition(json!([{ "action": "command", "value": "save-all" }])),
    )
    .await;
    let foreign_path = format!("{path}/{}", foreign["id"].as_str().unwrap());
    let rename = admin
        .call(
            Method::PUT,
            &foreign_path,
            Some(&token),
            Some(json!({
                "name": "Renommée",
                "cron": "0 4 * * *",
                "timezone": "UTC",
                "steps": foreign["steps"],
            })),
        )
        .await;
    assert_eq!(rename.status, StatusCode::FORBIDDEN, "{}", rename.json);
    let run = admin
        .call(
            Method::POST,
            &format!("{foreign_path}/run"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(run.status, StatusCode::FORBIDDEN, "{}", run.json);
}

#[tokio::test]
async fn reading_schedules_is_not_managing_them() {
    let setup = server("offline").await;
    let admin = &setup.platform.admin;
    let account = admin
        .account("viewer@example.com", &[Permission::ServersSchedulesRead])
        .await;
    let token = admin.login(&account).await.access_token;
    let path = format!("/servers/{}/schedules", setup.id);

    let list = admin.call(Method::GET, &path, Some(&token), None).await;
    assert_eq!(list.status, StatusCode::OK);
    let created = admin
        .call(
            Method::POST,
            &path,
            Some(&token),
            Some(definition(json!([{ "action": "backup" }]))),
        )
        .await;
    assert_eq!(created.status, StatusCode::FORBIDDEN);
    assert_eq!(
        created.json["params"]["permission"],
        "servers.schedules.manage"
    );
}

#[tokio::test]
async fn steps_run_in_order_as_the_system_and_the_run_is_recorded() {
    let mut setup = server("running").await;
    let schedule = create(
        &setup,
        definition(json!([
            { "action": "command", "value": "say restarting" },
            { "action": "power", "value": "restart" },
        ])),
    )
    .await;
    let due = next_run(&schedule);

    // `run_due` rend la main une fois l'exécution lancée : le nœud répond ensuite.
    let handles = scheduler::run_due(&setup.platform.admin.servers, due)
        .await
        .unwrap();
    let command = setup.daemon.accept().await;
    assert_eq!(command["type"], "server.command");
    assert_eq!(command["payload"]["line"], "say restarting");
    let restart = setup.daemon.accept().await;
    assert_eq!(restart["type"], "server.power");
    assert_eq!(restart["payload"]["action"], "restart");
    for handle in handles {
        handle.await.unwrap();
    }
    let history = runs(&setup, &schedule).await;
    assert_eq!(history[0]["outcome"], "succeeded", "{history:?}");
    assert_eq!(history[0]["trigger"], "schedule");

    let admin = &setup.platform.admin;
    let command_event = admin.last_audit("server.command").await;
    assert_eq!(command_event.actor_kind, "system");
    assert_eq!(
        command_event.actor_id.map(|id| id.to_string()),
        schedule["id"].as_str().map(str::to_owned)
    );
    let run_event = admin.last_audit("server.schedule_run").await;
    assert_eq!(run_event.outcome, "success");
    assert_eq!(run_event.details["outcome"], "succeeded");

    // L'échéance suivante est le lendemain.
    let after = setup
        .platform
        .call(
            Method::GET,
            &format!(
                "/servers/{}/schedules/{}",
                setup.id,
                schedule["id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(next_run(&after.json) - due, time::Duration::days(1));
}

#[tokio::test]
async fn a_backup_step_waits_for_the_backup_before_the_next_step() {
    let mut setup = server("running").await;
    let schedule = create(
        &setup,
        definition(json!([
            { "action": "backup", "value": "nuit" },
            { "action": "command", "value": "say backed up" },
        ])),
    )
    .await;
    let handles = scheduler::run_due(&setup.platform.admin.servers, next_run(&schedule))
        .await
        .unwrap();

    let request = setup.daemon.accept_backup().await;
    assert_eq!(request["type"], "backup.create");
    // Tant que le nœud n'a pas fini, la commande n'est pas envoyée.
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let backup_id = request["payload"]["backup_id"].clone();
    setup
        .daemon
        .event(
            "backup.done",
            json!({
                "server_id": setup.id,
                "backup_id": backup_id,
                "status": "ok",
                "size": 10,
                "sha256": "324fb9b3d3a59324cb5d96154f4fbc41c8f2d926f9bb2da1702f0230bd1b88d6",
            }),
        )
        .await;
    let command = setup.daemon.accept().await;
    assert_eq!(command["payload"]["line"], "say backed up");
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(runs(&setup, &schedule).await[0]["outcome"], "succeeded");
}

#[tokio::test]
async fn a_failing_step_stops_the_run_and_names_its_cause() {
    let setup = server("offline").await;
    let schedule = create(
        &setup,
        definition(json!([
            { "action": "command", "value": "say hi" },
            { "action": "command", "value": "never sent" },
        ])),
    )
    .await;

    run_due(&setup, next_run(&schedule)).await;

    let history = runs(&setup, &schedule).await;
    assert_eq!(history[0]["outcome"], "failed");
    assert_eq!(history[0]["failed_step"], 0);
    assert_eq!(history[0]["reason"], "server_not_running");
    let event = setup.platform.admin.last_audit("server.schedule_run").await;
    assert_eq!(event.outcome, "failure");
}

#[tokio::test]
async fn runs_are_skipped_when_late_offline_or_already_running() {
    let setup = server("offline").await;
    let only_online = create(
        &setup,
        json!({
            "name": "Annonce",
            "cron": "*/5 * * * *",
            "timezone": "UTC",
            "only_when_online": true,
            "steps": [{ "action": "command", "value": "say hi" }],
        }),
    )
    .await;
    let due = next_run(&only_online);

    // Plus de 5 min de retard (API arrêtée) : sautée, sans rattrapage.
    run_due(&setup, due + time::Duration::minutes(6)).await;
    assert_eq!(runs(&setup, &only_online).await[0]["reason"], "late");

    // Serveur arrêté : sautée.
    let schedule = setup
        .platform
        .call(
            Method::GET,
            &format!(
                "/servers/{}/schedules/{}",
                setup.id,
                only_online["id"].as_str().unwrap()
            ),
            None,
        )
        .await
        .json;
    run_due(&setup, next_run(&schedule)).await;
    let history = runs(&setup, &only_online).await;
    assert_eq!(history[0]["outcome"], "skipped");
    assert_eq!(history[0]["reason"], "offline");

    // Déjà en cours : sautée.
    let id = Uuid::parse_str(only_online["id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE server_schedules SET running_since = now() WHERE id = $1")
        .bind(id)
        .execute(&setup.platform.api.pool)
        .await
        .unwrap();
    let schedule = setup
        .platform
        .call(
            Method::GET,
            &format!("/servers/{}/schedules/{id}", setup.id),
            None,
        )
        .await
        .json;
    assert_eq!(schedule["running"], true);
    run_due(&setup, next_run(&schedule)).await;
    assert_eq!(runs(&setup, &only_online).await[0]["reason"], "overlap");
}

#[tokio::test]
async fn two_engines_never_run_the_same_schedule_twice() {
    let setup = server("offline").await;
    let schedule = create(
        &setup,
        definition(json!([{ "action": "command", "value": "say hi" }])),
    )
    .await;
    let due = next_run(&schedule);
    let state = &setup.platform.admin.servers;

    let (first, second) = tokio::join!(
        scheduler::run_due(state, due),
        scheduler::run_due(state, due)
    );
    let handles: Vec<_> = first.unwrap().into_iter().chain(second.unwrap()).collect();
    assert_eq!(handles.len(), 1);
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(runs(&setup, &schedule).await.len(), 1);
}

#[tokio::test]
async fn a_schedule_can_be_run_now_but_not_twice_at_once() {
    let mut setup = server("running").await;
    let schedule = create(
        &setup,
        definition(json!([{ "action": "command", "value": "say now" }])),
    )
    .await;
    let path = format!(
        "/servers/{}/schedules/{}/run",
        setup.id,
        schedule["id"].as_str().unwrap()
    );

    let started = setup.platform.call(Method::POST, &path, None).await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.json);
    assert_eq!(started.json["trigger"], "manual");
    assert_eq!(started.json["outcome"], "running");
    let again = setup.platform.call(Method::POST, &path, None).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json["code"], "schedule_running");

    let command = setup.daemon.accept().await;
    assert_eq!(command["payload"]["line"], "say now");
    setup
        .platform
        .admin
        .last_audit("server.schedule_started")
        .await;
}

#[tokio::test]
async fn deleting_a_schedule_forgets_it() {
    let setup = server("offline").await;
    let schedule = create(&setup, definition(json!([{ "action": "backup" }]))).await;
    let path = format!(
        "/servers/{}/schedules/{}",
        setup.id,
        schedule["id"].as_str().unwrap()
    );

    let deleted = setup.platform.call(Method::DELETE, &path, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let gone = setup.platform.call(Method::GET, &path, None).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.json["code"], "schedule_not_found");
    setup
        .platform
        .admin
        .last_audit("server.schedule_deleted")
        .await;
}

/// Archive de `minecraft-java` en `version`, manifest retouché par `edit`.
fn minecraft_version(version: &str, edit: impl Fn(String) -> String) -> Vec<u8> {
    use std::io::Write;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs/minecraft-java");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    // Fins de ligne normalisées : un checkout Windows du sous-module peut être en CRLF.
    let manifest = std::fs::read_to_string(dir.join("prefab.yaml"))
        .unwrap()
        .replace("\r\n", "\n")
        .replace("version: 1.4.0", &format!("version: {version}"));
    writer.start_file("prefab.yaml", options).unwrap();
    writer.write_all(edit(manifest).as_bytes()).unwrap();
    for lang in ["en", "fr"] {
        let text = std::fs::read_to_string(dir.join(format!("locales/{lang}.json"))).unwrap();
        writer
            .start_file(format!("locales/{lang}.json"), options)
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

async fn import(setup: &Setup, bytes: Vec<u8>) {
    let imported = setup
        .platform
        .admin
        .call_bytes(
            Method::POST,
            "/prefabs/import",
            &setup.platform.token,
            bytes,
            true,
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", imported.json);
}

#[tokio::test]
async fn an_upgrade_step_backs_up_then_moves_to_the_latest_published_version() {
    let mut setup = server("running").await;
    // 1.5.0 publiée, 1.6.0 en brouillon : seule la publiée est choisie d'office.
    import(&setup, minecraft_version("1.5.0", |m| m)).await;
    import(
        &setup,
        minecraft_version("1.6.0", |m| {
            m.replace("tags: [", "x-status: \"draft: later\"\ntags: [")
        }),
    )
    .await;
    let schedule = create(
        &setup,
        definition(json!([{ "action": "upgrade" }, { "action": "power", "value": "restart" }])),
    )
    .await;
    assert_eq!(schedule["steps"][0]["action"], "upgrade");
    assert!(schedule["steps"][0]["value"].is_null());

    let handles = scheduler::run_due(&setup.platform.admin.servers, next_run(&schedule))
        .await
        .unwrap();
    // Sauvegardes activées : la sauvegarde part d'abord et est attendue.
    let request = setup.daemon.accept_backup().await;
    setup
        .daemon
        .event(
            "backup.done",
            json!({
                "server_id": setup.id,
                "backup_id": request["payload"]["backup_id"],
                "status": "ok",
                "size": 10,
                "sha256": "324fb9b3d3a59324cb5d96154f4fbc41c8f2d926f9bb2da1702f0230bd1b88d6",
            }),
        )
        .await;
    let deploy = setup.daemon.accept().await;
    assert_eq!(deploy["type"], "server.deploy");
    // L'étape attend la fin du redéploiement avant le redémarrage qui suit.
    setup.daemon.report(&setup.id, "running").await;
    let restart = setup.daemon.accept().await;
    assert_eq!(restart["type"], "server.power");
    for handle in handles {
        handle.await.unwrap();
    }

    let server = setup
        .platform
        .call(Method::GET, &format!("/servers/{}", setup.id), None)
        .await;
    assert_eq!(server.json["prefab"]["version"], "1.5.0");
    let history = runs(&setup, &schedule).await;
    assert_eq!(history[0]["outcome"], "succeeded", "{history:?}");
    let upgraded = setup.platform.admin.last_audit("server.upgraded").await;
    assert_eq!(upgraded.outcome, "success");
    assert_eq!(upgraded.details["to"], "1.5.0");
    assert_eq!(upgraded.details["backup"], true);

    // Déjà à jour : la tâche réussit sans rien envoyer au nœud.
    let again = scheduler::run_due(
        &setup.platform.admin.servers,
        next_run(&schedule) + time::Duration::days(1),
    )
    .await
    .unwrap();
    let restart = setup.daemon.accept().await;
    assert_eq!(restart["type"], "server.power");
    for handle in again {
        handle.await.unwrap();
    }
    let history = runs(&setup, &schedule).await;
    assert_eq!(history.len(), 2);
    assert!(
        history.iter().all(|run| run["outcome"] == "succeeded"),
        "{history:?}"
    );
}

#[tokio::test]
async fn an_upgrade_step_names_its_blocker_and_needs_the_upgrade_permission() {
    let setup = server("running").await;
    // Seule version plus récente : un brouillon, hors nœud de test.
    import(
        &setup,
        minecraft_version("1.6.0", |m| {
            m.replace("tags: [", "x-status: \"draft: later\"\ntags: [")
        }),
    )
    .await;
    let invalid = setup
        .platform
        .call(
            Method::POST,
            &format!("/servers/{}/schedules", setup.id),
            Some(definition(
                json!([{ "action": "upgrade", "value": "not-a-version" }]),
            )),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.json["fields"][0]["field"], "steps.0.value");

    let schedule = create(
        &setup,
        definition(json!([{ "action": "upgrade", "value": "1.6.0" }])),
    )
    .await;
    run_due(&setup, next_run(&schedule)).await;
    let history = runs(&setup, &schedule).await;
    assert_eq!(history[0]["outcome"], "failed", "{history:?}");
    assert_eq!(history[0]["failed_step"], 0);
    assert_eq!(history[0]["reason"], "upgrade_blocked");

    // Sans `servers.upgrade`, pas d'étape de mise à jour.
    let admin = &setup.platform.admin;
    let limited = admin
        .account(
            "ops@example.com",
            &[
                Permission::ServersRead,
                Permission::ServersSchedulesRead,
                Permission::ServersSchedulesManage,
                Permission::ServersPower,
            ],
        )
        .await;
    let token = admin.login(&limited).await.access_token;
    let refused = admin
        .call(
            Method::POST,
            &format!("/servers/{}/schedules", setup.id),
            Some(&token),
            Some(definition(json!([{ "action": "upgrade" }]))),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.json);
    assert_eq!(refused.json["params"]["permission"], "servers.upgrade");
}
