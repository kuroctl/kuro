//! Commandes de l'API sur les serveurs : `server.deploy`, `server.delete`, `server.power`,
//! `server.sync`, `server.limits`, `server.suspend`.

use std::collections::BTreeMap;

use kuro_protocol::api::server::{
    FileContent, FileFormat, FileSpec, HealthcheckSpec, LifecycleSpec, Limits, PortProtocol,
    PortSpec, PowerAction, PublishedPort, ReadySpec, ServerAction, ServerDelete, ServerDeploy,
    ServerLimits, ServerPower, ServerSuspend, ServerSync, ServiceSpec, StopSignal, StopSpec,
    VolumeSpec, WebRoute,
};
use kuro_protocol::{ApiMessage, Envelope, decode};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";

fn server_id() -> Uuid {
    Uuid::parse_str(SERVER_ID).unwrap()
}

fn service(name: &str) -> ServiceSpec {
    ServiceSpec {
        name: name.to_owned(),
        image: "docker.io/library/alpine:3.20".to_owned(),
        command: Vec::new(),
        env: BTreeMap::new(),
        ports: Vec::new(),
        volumes: Vec::new(),
        writable: Vec::new(),
        user: "1000:1000".to_owned(),
        limits: Limits {
            memory_bytes: 268_435_456,
            cpu_millis: 500,
            pids: 256,
        },
        depends_on: Vec::new(),
        healthcheck: None,
    }
}

#[test]
fn server_deploy_wire_format() {
    let mut server = service("server");
    server.command = vec!["--nogui".to_owned()];
    server.env = BTreeMap::from([("EULA".to_owned(), "TRUE".to_owned())]);
    server.ports = vec![
        PortSpec {
            name: "game".to_owned(),
            container_port: 25565,
            protocol: PortProtocol::Tcp,
            published: Some(PublishedPort {
                host_ip: "0.0.0.0".to_owned(),
                host_port: 25570,
            }),
        },
        PortSpec {
            name: "rcon".to_owned(),
            container_port: 25575,
            protocol: PortProtocol::Tcp,
            published: None,
        },
    ];
    server.volumes = vec![VolumeSpec {
        name: "data".to_owned(),
        path: "/data".to_owned(),
    }];
    server.writable = vec!["/tmp".to_owned()];
    server.depends_on = vec!["db".to_owned()];
    server.healthcheck = Some(HealthcheckSpec {
        command: vec!["true".to_owned()],
        interval_secs: 10,
        retries: 3,
    });

    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerDeploy(ServerDeploy {
            server_id: server_id(),
            services: vec![server, service("db")],
            files: vec![
                FileSpec {
                    service: "server".to_owned(),
                    path: "/data/server.properties".to_owned(),
                    format: FileFormat::Properties,
                    content: FileContent::ManagedKeys {
                        keys: BTreeMap::from([("motd".to_owned(), serde_json::json!("Hello"))]),
                    },
                },
                FileSpec {
                    service: "server".to_owned(),
                    path: "/data/Game.ini".to_owned(),
                    format: FileFormat::Ini,
                    content: FileContent::User { content: None },
                },
                FileSpec {
                    service: "server".to_owned(),
                    path: "/data/motd.txt".to_owned(),
                    format: FileFormat::Text,
                    content: FileContent::Template {
                        content: "Welcome".to_owned(),
                    },
                },
            ],
            lifecycle: LifecycleSpec {
                service: "server".to_owned(),
                ready: Some(ReadySpec::Console(r"Done \(".to_owned())),
                stop: StopSpec::Console("stop".to_owned()),
                stop_timeout_secs: 60,
            },
            install: None,
            disk_bytes: 10_737_418_240,
            suspended: true,
            defer: false,
            web: Vec::new(),
        }),
    };

    let limits = json!({ "memory_bytes": 268_435_456, "cpu_millis": 500, "pids": 256 });
    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.deploy",
            "payload": {
                "server_id": SERVER_ID,
                "services": [
                    {
                        "name": "server",
                        "image": "docker.io/library/alpine:3.20",
                        "command": ["--nogui"],
                        "env": { "EULA": "TRUE" },
                        "ports": [
                            {
                                "name": "game",
                                "container_port": 25565,
                                "protocol": "tcp",
                                "published": { "host_ip": "0.0.0.0", "host_port": 25570 }
                            },
                            { "name": "rcon", "container_port": 25575, "protocol": "tcp" }
                        ],
                        "volumes": [{ "name": "data", "path": "/data" }],
                        "writable": ["/tmp"],
                        "user": "1000:1000",
                        "limits": limits,
                        "depends_on": ["db"],
                        "healthcheck": { "command": ["true"], "interval_secs": 10, "retries": 3 }
                    },
                    {
                        "name": "db",
                        "image": "docker.io/library/alpine:3.20",
                        "user": "1000:1000",
                        "limits": limits
                    }
                ],
                "files": [
                    {
                        "service": "server",
                        "path": "/data/server.properties",
                        "format": "properties",
                        "mode": "managed-keys",
                        "keys": { "motd": "Hello" }
                    },
                    { "service": "server", "path": "/data/Game.ini", "format": "ini", "mode": "user" },
                    {
                        "service": "server",
                        "path": "/data/motd.txt",
                        "format": "text",
                        "mode": "template",
                        "content": "Welcome"
                    }
                ],
                "lifecycle": {
                    "service": "server",
                    "ready": { "console": "Done \\(" },
                    "stop": { "console": "stop" },
                    "stop_timeout_secs": 60
                },
                "disk_bytes": 10_737_418_240_u64,
                "suspended": true
            }
        }),
    );
}

#[test]
fn deploy_from_an_older_api_has_no_quota_and_is_not_suspended() {
    let text = format!(
        r#"{{"id":"{ID}","type":"server.deploy","payload":{{"server_id":"{SERVER_ID}",
            "services":[],"lifecycle":{{"service":"server","stop":{{"signal":"SIGTERM"}},
            "stop_timeout_secs":10}}}}}}"#
    );

    let envelope = decode::<ApiMessage>(&text).unwrap();
    let ApiMessage::ServerDeploy(deploy) = envelope.message else {
        panic!("expected server.deploy");
    };
    assert_eq!(deploy.disk_bytes, 0);
    assert!(!deploy.suspended);
}

#[test]
fn a_web_route_is_carried_by_the_deploy_and_absent_without_a_site() {
    let mut deploy = ServerDeploy {
        server_id: server_id(),
        services: vec![service("web")],
        files: Vec::new(),
        lifecycle: LifecycleSpec {
            service: "web".to_owned(),
            ready: None,
            stop: StopSpec::Signal(StopSignal::Term),
            stop_timeout_secs: 20,
        },
        install: None,
        disk_bytes: 0,
        suspended: false,
        defer: false,
        web: vec![WebRoute {
            host: "abcdefg234.web.kuroctl.test".to_owned(),
            service: "web".to_owned(),
            container_port: 80,
        }],
    };
    let encoded = serde_json::to_value(&deploy).unwrap();
    assert_eq!(
        encoded["web"],
        json!([{ "host": "abcdefg234.web.kuroctl.test", "service": "web", "container_port": 80 }])
    );
    let decoded: ServerDeploy = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, deploy);

    // Sans site, et pour une API antérieure qui ne connaît pas le champ.
    deploy.web.clear();
    let mut encoded = serde_json::to_value(&deploy).unwrap();
    assert!(encoded.get("web").is_none());
    encoded.as_object_mut().unwrap().remove("web");
    let decoded: ServerDeploy = serde_json::from_value(encoded).unwrap();
    assert!(decoded.web.is_empty());
}

#[test]
fn managed_keys_keep_their_types_and_strings_keep_their_wire_format() {
    let keys = BTreeMap::from([
        ("motd".to_owned(), json!("hello")),
        ("gateway.port".to_owned(), json!(8080)),
        ("debug".to_owned(), json!(false)),
        ("trusted".to_owned(), json!(["10.0.0.0/8", "172.16.0.0/12"])),
    ]);
    let content = FileContent::ManagedKeys { keys: keys.clone() };
    let encoded = serde_json::to_value(&content).unwrap();
    assert_eq!(
        encoded,
        json!({ "mode": "managed-keys", "keys": {
            "motd": "hello", "gateway.port": 8080, "debug": false,
            "trusted": ["10.0.0.0/8", "172.16.0.0/12"]
        } })
    );
    let decoded: FileContent = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, content);
}

#[test]
fn an_action_and_its_result_round_trip() {
    use kuro_protocol::DaemonMessage;
    use kuro_protocol::daemon::server::ActionResult;

    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerAction(ServerAction {
            server_id: server_id(),
            action: "fix-owner".to_owned(),
            service: "web".to_owned(),
            command: vec![
                "chown".to_owned(),
                "-R".to_owned(),
                "33:33".to_owned(),
                "/var/www/html".to_owned(),
            ],
            timeout_secs: 120,
        }),
    };
    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.action",
            "payload": {
                "server_id": SERVER_ID, "action": "fix-owner", "service": "web",
                "command": ["chown", "-R", "33:33", "/var/www/html"], "timeout_secs": 120
            }
        }),
    );

    let result = Envelope {
        id: id(ID),
        reply_to: Some(id(ID)),
        message: DaemonMessage::ActionResult(ActionResult {
            server_id: server_id(),
            exit_code: 3,
            output: "done\n".to_owned(),
            truncated: false,
        }),
    };
    let encoded = serde_json::to_value(&result).unwrap();
    assert_eq!(encoded["type"], "action.result");
    assert_eq!(
        encoded["payload"],
        json!({ "server_id": SERVER_ID, "exit_code": 3, "output": "done\n" })
    );
    let decoded: Envelope<DaemonMessage> = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, result);
}

#[test]
fn stop_signals_and_the_image_signal_round_trip() {
    for (spec, wire) in [
        (
            StopSpec::Signal(StopSignal::Winch),
            json!({ "signal": "SIGWINCH" }),
        ),
        (
            StopSpec::Signal(StopSignal::Hup),
            json!({ "signal": "SIGHUP" }),
        ),
        (StopSpec::Image, json!("image")),
        (
            StopSpec::Console("stop".to_owned()),
            json!({ "console": "stop" }),
        ),
    ] {
        let encoded = serde_json::to_value(&spec).unwrap();
        assert_eq!(encoded, wire);
        let decoded: StopSpec = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, spec);
    }
    // Un signal hors liste n'entre jamais.
    assert!(serde_json::from_value::<StopSpec>(json!({ "signal": "SIGKILL" })).is_err());
}

#[test]
fn server_limits_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerLimits(ServerLimits {
            server_id: server_id(),
            services: BTreeMap::from([(
                "server".to_owned(),
                Limits {
                    memory_bytes: 4_294_967_296,
                    cpu_millis: 2000,
                    pids: 512,
                },
            )]),
            disk_bytes: 0,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.limits",
            "payload": {
                "server_id": SERVER_ID,
                "services": {
                    "server": { "memory_bytes": 4_294_967_296_u64, "cpu_millis": 2000, "pids": 512 }
                },
                "disk_bytes": 0
            }
        }),
    );
}

#[test]
fn server_suspend_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerSuspend(ServerSuspend {
            server_id: server_id(),
            suspended: true,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.suspend",
            "payload": { "server_id": SERVER_ID, "suspended": true }
        }),
    );
}

#[test]
fn lifecycle_with_healthcheck_and_signal() {
    let lifecycle = LifecycleSpec {
        service: "web".to_owned(),
        ready: Some(ReadySpec::Healthcheck),
        stop: StopSpec::Signal(StopSignal::Term),
        stop_timeout_secs: 20,
    };

    assert_eq!(
        serde_json::to_value(&lifecycle).unwrap(),
        json!({
            "service": "web",
            "ready": "healthcheck",
            "stop": { "signal": "SIGTERM" },
            "stop_timeout_secs": 20
        })
    );
}

#[test]
fn server_delete_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerDelete(ServerDelete {
            server_id: server_id(),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "server.delete", "payload": { "server_id": SERVER_ID } }),
    );
}

#[test]
fn server_power_wire_format() {
    for (action, wire) in [
        (PowerAction::Start, "start"),
        (PowerAction::Stop, "stop"),
        (PowerAction::Restart, "restart"),
        (PowerAction::Kill, "kill"),
    ] {
        let envelope = Envelope {
            id: id(ID),
            reply_to: None,
            message: ApiMessage::ServerPower(ServerPower {
                server_id: server_id(),
                action,
            }),
        };

        assert_wire_format(
            &envelope,
            json!({
                "id": ID,
                "type": "server.power",
                "payload": { "server_id": SERVER_ID, "action": wire }
            }),
        );
    }
}

#[test]
fn server_sync_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerSync(ServerSync {
            server_ids: vec![server_id()],
        }),
    };

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "server.sync", "payload": { "server_ids": [SERVER_ID] } }),
    );
}
