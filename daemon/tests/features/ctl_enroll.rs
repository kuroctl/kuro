//! `kuro-daemonctl enroll` : règles appliquées avant tout appel réseau.
//! Le parcours complet avec l'API est couvert par les tests de l'API.

use std::process::{Command, Output};

use tempfile::TempDir;

const FINGERPRINT: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn enroll(config_dir: &TempDir, extra: &[&str], token: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kuro-daemonctl"));
    command
        .arg("--config")
        .arg(config_dir.path().join("daemon.toml"))
        .args([
            "enroll",
            "--api",
            "https://127.0.0.1:1",
            "--ca-fingerprint",
            FINGERPRINT,
        ])
        .args(extra)
        .env_remove("KURO_DAEMON_CONFIG")
        .env_remove("KURO_ENROLL_TOKEN");
    if let Some(token) = token {
        command.env("KURO_ENROLL_TOKEN", token);
    }
    command.output().unwrap()
}

#[test]
fn token_is_required() {
    let dir = tempfile::tempdir().unwrap();

    let output = enroll(&dir, &[], None);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("KURO_ENROLL_TOKEN"));
}

#[test]
fn token_cannot_be_passed_as_an_argument() {
    let dir = tempfile::tempdir().unwrap();

    let output = enroll(&dir, &["--token", "kuro_enr_secret"], None);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));
}

#[test]
fn token_can_be_read_from_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let token_file = dir.path().join("token");
    std::fs::write(&token_file, "kuro_enr_secret\n").unwrap();
    std::fs::write(dir.path().join("node.key"), "existing").unwrap();

    let output = enroll(&dir, &["--token-file", token_file.to_str().unwrap()], None);

    // Le token a été lu : l'échec vient de la règle suivante (hôte déjà enrôlé).
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already enrolled"));
}

#[test]
fn already_enrolled_host_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("node.key"), "existing").unwrap();

    let output = enroll(&dir, &[], Some("kuro_enr_secret"));

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already enrolled"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("node.key")).unwrap(),
        "existing"
    );
}

#[test]
fn plain_http_api_url_is_refused() {
    let dir = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kuro-daemonctl"))
        .arg("--config")
        .arg(dir.path().join("daemon.toml"))
        .args([
            "enroll",
            "--api",
            "http://127.0.0.1:8443",
            "--ca-fingerprint",
            FINGERPRINT,
        ])
        .env("KURO_ENROLL_TOKEN", "kuro_enr_secret")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("https://"));
}
