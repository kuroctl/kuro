//! Plateforme complète pour les serveurs : REST et listener daemon sur la même base et le même
//! registre, prefabs officiels importés, super-admin connecté, et faux daemons pilotés à la main.

use std::path::Path;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use kuro_api::prefabs::application::import_dir;
use kuro_daemon::link::WsStream;
use serde_json::{Value, json};
use uuid::Uuid;

use super::admin::TestAdmin;
use super::http::TestResponse;
use super::link::{TestApi, TestNode, recv, say_hello, send};

const WAIT: Duration = Duration::from_secs(10);

pub struct Platform {
    pub api: TestApi,
    pub admin: TestAdmin,
    pub token: String,
}

impl Platform {
    pub async fn start() -> Self {
        let api = TestApi::start_with_heartbeat_timeout(600).await;
        let prefabs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prefabs");
        let report = import_dir(&api.pool, &prefabs).await.unwrap();
        assert!(report.invalid.is_empty(), "{:?}", report.invalid);
        let admin = TestAdmin::on(
            api.pool.clone(),
            api.links.clone(),
            api.console.clone(),
            false,
        );
        api.events.attach(admin.servers.clone());
        let root = admin.super_admin("root@example.com").await;
        let token = admin.login(&root).await.access_token;
        Self { api, admin, token }
    }

    pub async fn call(&self, method: Method, path: &str, body: Option<Value>) -> TestResponse {
        self.admin.call(method, path, Some(&self.token), body).await
    }

    /// Nœud enrôlé, connecté par un faux daemon, avec des ports libres.
    pub async fn online_node(&self, name: &str, ports: &str) -> FakeDaemon {
        let node = self.api.enroll_node(name).await;
        let mut ws = self.api.connect(&node).await;
        say_hello(&mut ws).await;
        let created = self
            .call(
                Method::POST,
                &format!("/nodes/{}/allocations", node.id),
                Some(json!({ "ports": ports })),
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.json);
        FakeDaemon { node, ws }
    }

    /// Déploie `minecraft-java` ; le faux daemon accepte la commande. Retourne le serveur et la
    /// commande reçue.
    pub async fn deploy_minecraft(
        &self,
        daemon: &mut FakeDaemon,
        settings: Value,
    ) -> (Value, Value) {
        let (response, command) = tokio::join!(
            self.call(
                Method::POST,
                "/servers",
                Some(json!({
                    "name": "Survie",
                    "node_id": daemon.node.id.as_uuid(),
                    "prefab_id": "minecraft-java",
                    "memory_mb": 2048,
                    "cpu_percent": 200,
                    "disk_mb": 10240,
                    "settings": settings,
                })),
            ),
            daemon.accept(),
        );
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.json);
        (response.json, command)
    }

    /// Attend que le serveur atteigne `state` (les états rapportés sont traités en différé).
    pub async fn wait_for_state(&self, id: &str, state: &str) -> Value {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let server = self
                .call(Method::GET, &format!("/servers/{id}"), None)
                .await;
            if server.json["state"] == state {
                return server.json;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "server stayed {}, expected {state}",
                server.json["state"]
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Daemon simulé sur le vrai lien mTLS : lit les commandes, répond, rapporte des états.
pub struct FakeDaemon {
    pub node: TestNode,
    pub ws: WsStream,
}

impl FakeDaemon {
    /// Prochaine commande de serveur (`server.*`), en ignorant les autres messages.
    pub async fn next_command(&mut self) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let message = recv(&mut self.ws).await.expect("link closed");
                if message["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("server."))
                {
                    return message;
                }
            }
        })
        .await
        .expect("no command received")
    }

    pub async fn reply(&mut self, command: &Value, result: Value) {
        send(
            &mut self.ws,
            json!({
                "id": Uuid::now_v7(),
                "reply_to": command["id"],
                "type": "command.result",
                "payload": result,
            }),
        )
        .await;
    }

    /// Réponse quelconque (`file.listing`…) à une requête.
    pub async fn answer(&mut self, request: &Value, kind: &str, payload: Value) {
        send(
            &mut self.ws,
            json!({
                "id": Uuid::now_v7(),
                "reply_to": request["id"],
                "type": kind,
                "payload": payload,
            }),
        )
        .await;
    }

    /// Prochaine requête de fichier (`file.*`).
    pub async fn next_file_request(&mut self) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let message = recv(&mut self.ws).await.expect("link closed");
                if message["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("file."))
                {
                    return message;
                }
            }
        })
        .await
        .expect("no file request received")
    }

    /// Prochaine requête de sauvegarde (`backup.*`).
    pub async fn next_backup_request(&mut self) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let message = recv(&mut self.ws).await.expect("link closed");
                if message["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("backup."))
                {
                    return message;
                }
            }
        })
        .await
        .expect("no backup request received")
    }

    /// Accepte un envoi de fichier (`file.upload` puis ses morceaux) ; retourne les morceaux.
    pub async fn accept_upload(&mut self) -> (Value, Vec<Value>) {
        let start = self.next_file_request().await;
        assert_eq!(start["type"], "file.upload", "{start}");
        self.reply(&start, json!({ "status": "ok" })).await;
        let mut chunks = Vec::new();
        loop {
            let chunk = self.next_file_request().await;
            assert_eq!(chunk["type"], "file.upload.chunk", "{chunk}");
            self.reply(&chunk, json!({ "status": "ok" })).await;
            let last = chunk["payload"]["eof"] == true;
            chunks.push(chunk);
            if last {
                return (start, chunks);
            }
        }
    }

    /// Prochaine requête de contenu (`content.*`).
    pub async fn next_content_request(&mut self) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                let message = recv(&mut self.ws).await.expect("link closed");
                if message["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("content."))
                {
                    return message;
                }
            }
        })
        .await
        .expect("no content request received")
    }

    /// Accepte la prochaine requête de contenu et la retourne.
    pub async fn accept_content(&mut self) -> Value {
        let request = self.next_content_request().await;
        self.reply(&request, json!({ "status": "ok" })).await;
        request
    }

    /// Accepte la prochaine requête de sauvegarde et la retourne.
    pub async fn accept_backup(&mut self) -> Value {
        let request = self.next_backup_request().await;
        self.reply(&request, json!({ "status": "ok" })).await;
        request
    }

    /// Événement spontané du nœud (`backup.done`…).
    pub async fn event(&mut self, kind: &str, payload: Value) {
        send(
            &mut self.ws,
            json!({ "id": Uuid::now_v7(), "type": kind, "payload": payload }),
        )
        .await;
    }

    /// Accepte la prochaine commande et la retourne.
    pub async fn accept(&mut self) -> Value {
        let command = self.next_command().await;
        self.reply(&command, json!({ "status": "ok" })).await;
        command
    }

    pub async fn report(&mut self, server_id: &str, state: &str) {
        send(
            &mut self.ws,
            json!({
                "id": Uuid::now_v7(),
                "type": "server.state",
                "payload": { "server_id": server_id, "state": state },
            }),
        )
        .await;
    }
}
