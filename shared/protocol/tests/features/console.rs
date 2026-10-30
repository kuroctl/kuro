//! Console et stats (ADR-0014) : `server.watch`, `server.command`, `console.line`,
//! `server.stats`, et les règles partagées par l'API et le daemon.

use kuro_protocol::api::{ServerCommand, ServerWatch};
use kuro_protocol::daemon::server::{CommandErrorCode, CommandResult};
use kuro_protocol::daemon::{ConsoleLine, ConsoleOutput, ServerStats};
use kuro_protocol::limits::{
    MAX_COMMAND_CHARS, MAX_CONSOLE_LINE_BYTES, is_valid_command, truncate_console_line,
};
use kuro_protocol::{ApiMessage, DaemonMessage, Envelope, decode};
use serde_json::json;
use uuid::Uuid;

use crate::support::{ID, REQUEST_ID, assert_wire_format, id};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";

fn server_id() -> Uuid {
    Uuid::parse_str(SERVER_ID).unwrap()
}

#[test]
fn server_watch_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerWatch(ServerWatch {
            server_id: server_id(),
            watch: true,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.watch",
            "payload": { "server_id": SERVER_ID, "watch": true }
        }),
    );
}

#[test]
fn server_command_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: ApiMessage::ServerCommand(ServerCommand {
            server_id: server_id(),
            line: "say hello".to_owned(),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.command",
            "payload": { "server_id": SERVER_ID, "line": "say hello" }
        }),
    );
}

#[test]
fn console_lines_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ConsoleLine(ConsoleOutput {
            server_id: server_id(),
            lines: vec![ConsoleLine {
                at_ms: 1_793_042_865_453,
                text: "\u{1b}[32mDone (3.2s)!\u{1b}[0m".to_owned(),
            }],
            dropped: 0,
        }),
    };

    // Aucune ligne perdue : le champ est omis.
    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "console.line",
            "payload": {
                "server_id": SERVER_ID,
                "lines": [{ "at_ms": 1_793_042_865_453_i64, "text": "\u{1b}[32mDone (3.2s)!\u{1b}[0m" }]
            }
        }),
    );
}

#[test]
fn console_lines_report_dropped_lines() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ConsoleLine(ConsoleOutput {
            server_id: server_id(),
            lines: Vec::new(),
            dropped: 42,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "console.line",
            "payload": { "server_id": SERVER_ID, "lines": [], "dropped": 42 }
        }),
    );
}

#[test]
fn server_stats_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: None,
        message: DaemonMessage::ServerStats(ServerStats {
            server_id: server_id(),
            cpu_millis: 1500,
            memory_bytes: 1_073_741_824,
            memory_limit_bytes: 2_147_483_648,
            network_rx_bytes: 1024,
            network_tx_bytes: 2048,
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "type": "server.stats",
            "payload": {
                "server_id": SERVER_ID,
                "cpu_millis": 1500,
                "memory_bytes": 1_073_741_824,
                "memory_limit_bytes": 2_147_483_648_u64,
                "network_rx_bytes": 1024,
                "network_tx_bytes": 2048
            }
        }),
    );
}

#[test]
fn not_running_refusal_wire_format() {
    let envelope = Envelope {
        id: id(ID),
        reply_to: Some(id(REQUEST_ID)),
        message: DaemonMessage::CommandResult(CommandResult::Error {
            code: CommandErrorCode::NotRunning,
            message: "server is not running".to_owned(),
        }),
    };

    assert_wire_format(
        &envelope,
        json!({
            "id": ID,
            "reply_to": REQUEST_ID,
            "type": "command.result",
            "payload": { "status": "error", "code": "not_running", "message": "server is not running" }
        }),
    );
}

#[test]
fn unknown_daemon_message_type_is_rejected() {
    let text = json!({ "id": ID, "type": "console.cleared", "payload": {} }).to_string();
    assert!(decode::<DaemonMessage>(&text).is_err());
}

#[test]
fn accepts_a_plain_command() {
    assert!(is_valid_command("say Bonjour à tous"));
    assert!(is_valid_command(&"a".repeat(MAX_COMMAND_CHARS)));
}

#[test]
fn rejects_empty_multiline_control_or_oversized_commands() {
    assert!(!is_valid_command(""));
    assert!(!is_valid_command("   "));
    assert!(!is_valid_command("stop\nop attacker"));
    assert!(!is_valid_command("say \u{1b}[2J"));
    assert!(!is_valid_command("a\tb"));
    assert!(!is_valid_command(&"a".repeat(MAX_COMMAND_CHARS + 1)));
}

#[test]
fn truncates_long_console_lines_on_a_char_boundary() {
    let line = "é".repeat(MAX_CONSOLE_LINE_BYTES);
    let truncated = truncate_console_line(line);
    assert!(truncated.len() <= MAX_CONSOLE_LINE_BYTES);
    assert!(truncated.chars().all(|c| c == 'é'));

    assert_eq!(truncate_console_line("short".to_owned()), "short");
}
