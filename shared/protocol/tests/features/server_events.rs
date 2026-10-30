//! Réponses et événements du daemon : `command.result`, `server.state`, `server.disk`.

use kuro_protocol::daemon::server::{
    CommandErrorCode, CommandResult, ServerDisk, ServerState, ServerStateReport,
};
use kuro_protocol::{DaemonMessage, Envelope, decode};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";

#[test]
fn accepted_command_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: DaemonMessage::CommandResult(CommandResult::Ok),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "command.result",
            "payload": { "status": "ok" }
        }),
    );
}

#[test]
fn refused_command_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: DaemonMessage::CommandResult(CommandResult::Error {
            code: CommandErrorCode::InvalidSpec,
            message: "service web runs as root".to_owned(),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "command.result",
            "payload": { "status": "error", "code": "invalid_spec", "message": "service web runs as root" }
        }),
    );
}

#[test]
fn unknown_command_error_code_is_tolerated() {
    let text = format!(
        r#"{{"id":"{ID}","reply_to":"{REQUEST_ID}","type":"command.result",
            "payload":{{"status":"error","code":"quota_exceeded","message":"x"}}}}"#
    );

    let envelope = decode::<DaemonMessage>(&text).unwrap();

    assert_eq!(
        envelope.message,
        DaemonMessage::CommandResult(CommandResult::Error {
            code: CommandErrorCode::Unknown,
            message: "x".to_owned(),
        })
    );
}

#[test]
fn server_state_wire_format() {
    for (state, wire) in [
        (ServerState::Deploying, "deploying"),
        (ServerState::Offline, "offline"),
        (ServerState::Starting, "starting"),
        (ServerState::Running, "running"),
        (ServerState::Stopping, "stopping"),
        (ServerState::Deleted, "deleted"),
    ] {
        let envelope = Envelope {
            id: id(ID),
            reply_to: None,
            message: DaemonMessage::ServerState(ServerStateReport {
                server_id: Uuid::parse_str(SERVER_ID).unwrap(),
                state,
                reason: None,
                web: None,
            }),
        };

        assert_wire_format(
            &envelope,
            json!({
                "id": ID,
                "type": "server.state",
                "payload": { "server_id": SERVER_ID, "state": wire }
            }),
        );
    }
}

#[test]
fn failed_state_carries_its_reason() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ServerState(ServerStateReport {
            server_id: Uuid::parse_str(SERVER_ID).unwrap(),
            state: ServerState::Failed,
            reason: Some("image pull failed".to_owned()),
            web: None,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.state",
            "payload": { "server_id": SERVER_ID, "state": "failed", "reason": "image pull failed" }
        }),
    );
}

#[test]
fn quota_and_suspension_refusals_wire_format() {
    for (code, wire) in [
        (CommandErrorCode::DiskQuotaExceeded, "disk_quota_exceeded"),
        (CommandErrorCode::Suspended, "suspended"),
    ] {
        let envelope = Envelope {
            id: id(ID),
            reply_to: Some(id(REQUEST_ID)),
            message: DaemonMessage::CommandResult(CommandResult::Error {
                code,
                message: "refused".to_owned(),
            }),
        };

        assert_wire_format(
            &envelope,
            json!({
                "id": ID,
                "reply_to": REQUEST_ID,
                "type": "command.result",
                "payload": { "status": "error", "code": wire, "message": "refused" }
            }),
        );
    }
}

#[test]
fn server_disk_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ServerDisk(ServerDisk {
            server_id: Uuid::parse_str(SERVER_ID).unwrap(),
            used_bytes: 1_073_741_824,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.disk",
            "payload": { "server_id": SERVER_ID, "used_bytes": 1_073_741_824 }
        }),
    );
}

#[test]
fn web_status_rides_with_the_state_and_an_older_daemon_sends_none() {
    use kuro_protocol::daemon::server::{CertificateState, ProxyState, WebStatus};

    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ServerState(ServerStateReport {
            server_id: Uuid::parse_str(SERVER_ID).unwrap(),
            state: ServerState::Running,
            reason: None,
            web: Some(WebStatus {
                proxy: ProxyState::Unavailable,
                certificate: CertificateState::Pending,
                message: Some("port 443 already in use".to_owned()),
            }),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.state",
            "payload": {
                "server_id": SERVER_ID,
                "state": "running",
                "web": { "proxy": "unavailable", "certificate": "pending", "message": "port 443 already in use" }
            }
        }),
    );

    let older = json!({
        "id": ID,
        "type": "server.state",
        "payload": { "server_id": SERVER_ID, "state": "running" }
    });
    let decoded: Envelope<DaemonMessage> = serde_json::from_value(older).unwrap();
    match decoded.message {
        DaemonMessage::ServerState(report) => assert_eq!(report.web, None),
        other => panic!("{other:?}"),
    }
}
