//! `kuro-daemonctl config check`, exécuté comme le ferait un opérateur.

use std::io::Write;
use std::process::{Command, Output};

use tempfile::NamedTempFile;

const VALID: &str = r#"
[api]
url = "wss://api.example.test/daemon"

[tls]
ca_cert = "/etc/kuro/ca.pem"
cert = "/etc/kuro/node.pem"
key = "/etc/kuro/node.key"
"#;

fn config_file(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file
}

fn config_check(path: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kuro-daemonctl"))
        .args(["config", "check", "--config"])
        .arg(path)
        .env_remove("KURO_DAEMON_CONFIG")
        .output()
        .unwrap()
}

#[test]
fn valid_config_passes() {
    let file = config_file(VALID);

    let output = config_check(file.path());

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("configuration OK"));
}

#[test]
fn plaintext_api_url_is_rejected() {
    let file = config_file(&VALID.replace("wss://", "ws://"));

    let output = config_check(file.path());

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wss://"));
}

#[test]
fn missing_tls_section_is_rejected() {
    let file = config_file("[api]\nurl = \"wss://api.example.test/daemon\"\n");

    let output = config_check(file.path());

    assert!(!output.status.success());
}

#[test]
fn missing_config_file_is_rejected() {
    let output = config_check(std::path::Path::new("does/not/exist.toml"));

    assert!(!output.status.success());
}
