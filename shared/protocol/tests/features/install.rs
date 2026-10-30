//! Installation (ADR-0017) : `server.install`, état `installing`, codes de refus.

use kuro_protocol::api::{
    InstallMode, InstallSpec, InstallStep, ServerInstall, SteamLogin, SteamPlatform,
};
use kuro_protocol::daemon::server::{
    CommandErrorCode, CommandResult, ServerState, ServerStateReport,
};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";
const SHA: &str = "324fb9b3d3a59324cb5d96154f4fbc41c8f2d926f9bb2da1702f0230bd1b88d6";

fn server_id() -> Uuid {
    Uuid::parse_str(SERVER_ID).unwrap()
}

#[test]
fn reinstall_wire_format_with_every_step() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerInstall(ServerInstall {
            server_id: server_id(),
            mode: InstallMode::Reinstall,
            install: Some(InstallSpec {
                service: "server".to_owned(),
                image: "docker.io/library/debian:bookworm-slim".to_owned(),
                steps: vec![
                    InstallStep::Download {
                        url: "https://terraria.org/server.zip".to_owned(),
                        sha256: SHA.to_owned(),
                        to: "/data/server.zip".to_owned(),
                    },
                    InstallStep::Extract {
                        from: "/data/server.zip".to_owned(),
                        to: "/data/server".to_owned(),
                        subdir: Some("1449/Linux".to_owned()),
                        executable: vec!["TerrariaServer.bin.x86_64".to_owned()],
                    },
                    InstallStep::Copy {
                        content: "maxplayers=8\n".to_owned(),
                        to: "/data/serverconfig.txt".to_owned(),
                    },
                    InstallStep::Remove {
                        path: "/data/server.zip".to_owned(),
                    },
                    InstallStep::Steamcmd {
                        app: 896_660,
                        dir: "/data/valheim".to_owned(),
                        beta: None,
                        validate: true,
                        platform: SteamPlatform::Linux,
                        login: None,
                    },
                    InstallStep::Script {
                        run: "echo done".to_owned(),
                        image: None,
                    },
                ],
            }),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.install",
            "payload": {
                "server_id": SERVER_ID,
                "mode": "reinstall",
                "install": {
                    "service": "server",
                    "image": "docker.io/library/debian:bookworm-slim",
                    "steps": [
                        { "download": { "url": "https://terraria.org/server.zip", "sha256": SHA, "to": "/data/server.zip" } },
                        { "extract": {
                            "from": "/data/server.zip", "to": "/data/server",
                            "subdir": "1449/Linux", "executable": ["TerrariaServer.bin.x86_64"]
                        } },
                        { "copy": { "content": "maxplayers=8\n", "to": "/data/serverconfig.txt" } },
                        { "remove": { "path": "/data/server.zip" } },
                        { "steamcmd": { "app": 896_660, "dir": "/data/valheim", "validate": true } },
                        { "script": { "run": "echo done" } }
                    ]
                }
            }
        }),
    );
}

#[test]
fn a_reset_without_install_steps_omits_them() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerInstall(ServerInstall {
            server_id: server_id(),
            mode: InstallMode::Reset,
            install: None,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "server.install", "payload": { "server_id": SERVER_ID, "mode": "reset" } }),
    );
}

#[test]
fn installing_state_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ServerState(ServerStateReport {
            server_id: server_id(),
            state: ServerState::Installing,
            reason: None,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({ "id": ID, "type": "server.state", "payload": { "server_id": SERVER_ID, "state": "installing" } }),
    );
}

#[test]
fn install_refusal_codes_wire_format() {
    for (code, text) in [
        (CommandErrorCode::DownloadFailed, "download_failed"),
        (CommandErrorCode::ChecksumMismatch, "checksum_mismatch"),
        (CommandErrorCode::InstallFailed, "install_failed"),
    ] {
        let envelope = Envelope {
            id: id(ID),
            reply_to: Some(id(REQUEST_ID)),
            message: DaemonMessage::CommandResult(CommandResult::Error {
                code,
                message: "x".to_owned(),
            }),
        };
        assert_wire_format(
            &envelope,
            json!({ "id": ID, "reply_to": REQUEST_ID, "type": "command.result",
                "payload": { "status": "error", "code": text, "message": "x" } }),
        );
    }
}

#[test]
fn a_first_install_travels_with_the_deployment() {
    let text = json!({
        "id": ID,
        "type": "server.deploy",
        "payload": {
            "server_id": SERVER_ID,
            "services": [],
            "lifecycle": { "service": "server", "stop": { "signal": "SIGTERM" }, "stop_timeout_secs": 10 },
            "install": { "service": "server", "image": "docker.io/library/debian:bookworm-slim", "steps": [
                { "remove": { "path": "/data/old" } }
            ] }
        }
    })
    .to_string();

    let envelope: Envelope<ApiMessage> = kuro_protocol::decode(&text).unwrap();

    let ApiMessage::ServerDeploy(deploy) = envelope.message else {
        panic!("not a deployment");
    };
    let install = deploy.install.unwrap();
    assert_eq!(
        install.steps,
        [InstallStep::Remove {
            path: "/data/old".to_owned()
        }]
    );
}

#[test]
fn a_windows_steamcmd_step_carries_its_platform() {
    let step = InstallStep::Steamcmd {
        app: 2_278_520,
        dir: "/data/server".to_owned(),
        beta: None,
        validate: true,
        platform: SteamPlatform::Windows,
        login: None,
    };

    let wire = serde_json::to_value(&step).unwrap();

    assert_eq!(
        wire,
        json!({ "steamcmd": { "app": 2_278_520, "dir": "/data/server", "validate": true, "platform": "windows" } })
    );
    assert_eq!(serde_json::from_value::<InstallStep>(wire).unwrap(), step);
}

#[test]
fn a_step_from_an_older_api_installs_linux_files() {
    // Une API antérieure à ADR-0032 n'envoie pas `platform`.
    let step: InstallStep = serde_json::from_value(json!({
        "steamcmd": { "app": 896_660, "dir": "/data/valheim", "validate": true }
    }))
    .unwrap();

    assert!(matches!(
        step,
        InstallStep::Steamcmd {
            platform: SteamPlatform::Linux,
            ..
        }
    ));
}

#[test]
fn a_steamcmd_step_with_an_account_carries_it_and_hides_the_password_from_debug() {
    let step = InstallStep::Steamcmd {
        app: 233_780,
        dir: "/data/server".to_owned(),
        beta: None,
        validate: false,
        platform: SteamPlatform::Linux,
        login: Some(SteamLogin {
            username: "kuro_servers".to_owned(),
            password: "s3cret-pass".to_owned(),
        }),
    };

    let wire = serde_json::to_value(&step).unwrap();
    assert_eq!(
        wire,
        json!({ "steamcmd": {
            "app": 233_780, "dir": "/data/server", "validate": false,
            "login": { "username": "kuro_servers", "password": "s3cret-pass" },
        } })
    );
    assert_eq!(serde_json::from_value::<InstallStep>(wire).unwrap(), step);

    // Journaux de l'API et du daemon : l'identifiant, jamais le mot de passe.
    let debug = format!("{step:?}");
    assert!(debug.contains("kuro_servers"), "{debug}");
    assert!(!debug.contains("s3cret-pass"), "{debug}");
}
