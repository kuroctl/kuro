//! Événements du cycle de vie (ADR-0051 § 6, tranche 6) : une extension reçoit création,
//! installation, suspension, transfert et suppression d'un serveur ; une extension en erreur
//! n'empêche ni l'action de Kuro, ni les autres extensions.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::future::BoxFuture;
use kuro_api::extension::{Extension, ExtensionFactory, HookError, ServerHooks};
use kuro_api::servers::domain::ServerEvent;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support::platform::{FakeDaemon, Platform};

const ALICE: &str = "shop:alice";
const BOB: &str = "shop:bob";

/// Extension de test qui garde les événements reçus.
#[derive(Default)]
struct Recorder(Mutex<Vec<ServerEvent>>);

impl ServerHooks for Recorder {
    fn on_server_event<'a>(
        &'a self,
        event: &'a ServerEvent,
    ) -> BoxFuture<'a, Result<(), HookError>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(event.clone());
            Ok(())
        })
    }
}

struct Failing;

impl ServerHooks for Failing {
    fn on_server_event<'a>(
        &'a self,
        _event: &'a ServerEvent,
    ) -> BoxFuture<'a, Result<(), HookError>> {
        Box::pin(async { Err(HookError("billing down".to_owned())) })
    }
}

fn listening(name: &'static str, hooks: Arc<dyn ServerHooks>) -> ExtensionFactory {
    ExtensionFactory::new(name, move |_| Ok(Extension::default().hooks(hooks.clone())))
}

impl Recorder {
    /// Attend l'événement voulu (diffusé dans une tâche à part), 5 s au plus.
    async fn wait_for(&self, expected: &ServerEvent) {
        for _ in 0..250 {
            if self.0.lock().unwrap().contains(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("missing {expected:?}, got {:?}", self.0.lock().unwrap());
    }
}

async fn create(platform: &Platform, daemon: &mut FakeDaemon, owner: Value) -> Uuid {
    let (response, _) = tokio::join!(
        platform.call(
            Method::POST,
            "/servers",
            Some(json!({
                "name": "Survie",
                "node_id": daemon.node.id.as_uuid(),
                "prefab_id": "minecraft-java",
                "owner_ref": owner,
            })),
        ),
        daemon.accept(),
    );
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
    response.json["id"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn an_extension_follows_the_whole_life_of_a_server() {
    let recorder = Arc::new(Recorder::default());
    let platform = Platform::start_with_extensions(vec![listening("shop", recorder.clone())]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let alice = || Some(ALICE.to_owned());

    let id = create(&platform, &mut daemon, json!(ALICE)).await;
    recorder
        .wait_for(&ServerEvent::Created {
            server_id: id,
            owner_ref: alice(),
        })
        .await;

    daemon.report(&id.to_string(), "installing").await;
    platform.wait_for_state(&id.to_string(), "installing").await;
    daemon.report(&id.to_string(), "offline").await;
    recorder
        .wait_for(&ServerEvent::Installed {
            server_id: id,
            owner_ref: alice(),
        })
        .await;

    let suspend = format!("/servers/{id}/suspend");
    let (suspended, _) = tokio::join!(platform.call(Method::POST, &suspend, None), daemon.accept());
    assert_eq!(suspended.status, StatusCode::OK, "{}", suspended.json);
    recorder
        .wait_for(&ServerEvent::Suspended {
            server_id: id,
            owner_ref: alice(),
        })
        .await;
    let unsuspend = format!("/servers/{id}/unsuspend");
    let (unsuspended, _) = tokio::join!(
        platform.call(Method::POST, &unsuspend, None),
        daemon.accept()
    );
    assert_eq!(unsuspended.status, StatusCode::OK, "{}", unsuspended.json);
    recorder
        .wait_for(&ServerEvent::Unsuspended {
            server_id: id,
            owner_ref: alice(),
        })
        .await;

    let moved = platform
        .call(
            Method::PUT,
            &format!("/servers/{id}/owner"),
            Some(json!({ "owner_ref": BOB })),
        )
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.json);
    recorder
        .wait_for(&ServerEvent::OwnerChanged {
            server_id: id,
            from: alice(),
            to: Some(BOB.to_owned()),
        })
        .await;

    let path = format!("/servers/{id}");
    let (deleted, _) = tokio::join!(platform.call(Method::DELETE, &path, None), daemon.accept());
    assert_eq!(deleted.status, StatusCode::ACCEPTED, "{}", deleted.json);
    daemon.report(&id.to_string(), "deleted").await;
    recorder
        .wait_for(&ServerEvent::Deleted {
            server_id: id,
            owner_ref: Some(BOB.to_owned()),
        })
        .await;
}

#[tokio::test]
async fn a_failed_installation_is_reported_with_its_reason() {
    let recorder = Arc::new(Recorder::default());
    let platform = Platform::start_with_extensions(vec![listening("shop", recorder.clone())]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = create(&platform, &mut daemon, json!(ALICE)).await;

    daemon.report(&id.to_string(), "installing").await;
    platform.wait_for_state(&id.to_string(), "installing").await;
    daemon.report(&id.to_string(), "failed").await;

    let failed = platform.wait_for_state(&id.to_string(), "failed").await;
    recorder
        .wait_for(&ServerEvent::InstallFailed {
            server_id: id,
            owner_ref: Some(ALICE.to_owned()),
            reason: failed["state_reason"].as_str().map(str::to_owned),
        })
        .await;
}

#[tokio::test]
async fn a_restart_after_installation_is_not_an_installation() {
    let recorder = Arc::new(Recorder::default());
    let platform = Platform::start_with_extensions(vec![listening("shop", recorder.clone())]).await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;
    let id = create(&platform, &mut daemon, Value::Null).await;
    daemon.report(&id.to_string(), "installing").await;
    platform.wait_for_state(&id.to_string(), "installing").await;
    daemon.report(&id.to_string(), "offline").await;
    recorder
        .wait_for(&ServerEvent::Installed {
            server_id: id,
            owner_ref: None,
        })
        .await;

    daemon.report(&id.to_string(), "starting").await;
    daemon.report(&id.to_string(), "running").await;
    platform.wait_for_state(&id.to_string(), "running").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let installed = recorder
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, ServerEvent::Installed { .. }))
        .count();
    assert_eq!(installed, 1);
}

#[tokio::test]
async fn a_failing_extension_neither_blocks_kuro_nor_the_other_extensions() {
    let recorder = Arc::new(Recorder::default());
    let platform = Platform::start_with_extensions(vec![
        listening("billing", Arc::new(Failing)),
        listening("shop", recorder.clone()),
    ])
    .await;
    let mut daemon = platform.online_node("node-1", "25565-25570").await;

    let id = create(&platform, &mut daemon, json!(ALICE)).await;

    recorder
        .wait_for(&ServerEvent::Created {
            server_id: id,
            owner_ref: Some(ALICE.to_owned()),
        })
        .await;
}
