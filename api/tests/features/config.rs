use std::io::Write;

use kuro_api::config::{Config, ConfigError};

const REQUIRED: &str = r#"
[database]
url = "postgres://user:pass@127.0.0.1/kuro"

[auth]
secret_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

[daemon]
ca_cert = "pki/ca.pem"
ca_key = "pki/ca.key"
cert = "pki/api.pem"
key = "pki/api.key"
"#;

fn load(extra: &str) -> Result<Config, ConfigError> {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "{extra}\n{REQUIRED}").unwrap();
    Config::load(Some(file.path()))
}

#[test]
fn minimal_config_uses_defaults() {
    let config = load("").unwrap();

    assert!(config.http.bind.ip().is_loopback());
    assert_eq!(config.daemon.bind.port(), 8443);
    assert_eq!(config.daemon.heartbeat_interval_secs, 15);
    assert_eq!(config.daemon.heartbeat_timeout_secs, 45);
    assert_eq!(config.enrollment.token_ttl_secs, 900);
    assert_eq!(config.enrollment.certificate_validity_days, 365);
}

#[test]
fn auth_defaults_are_secure() {
    let config = load("").unwrap();

    assert!(config.auth.require_totp);
    assert_eq!(config.auth.access_ttl_secs, 900);
    assert!(!format!("{:?}", config.auth).contains("0123456789abcdef"));
}

#[test]
fn malformed_secret_key_is_rejected() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(
        file,
        "{}",
        REQUIRED.replace("0123456789abcdef0123", "too-short")
    )
    .unwrap();

    let error = Config::load(Some(file.path())).unwrap_err();

    assert!(matches!(error, ConfigError::InvalidSecretKey(_)));
}

#[test]
fn access_tokens_must_expire_before_refresh_tokens() {
    let toml = REQUIRED.replace(
        "[auth]
",
        "[auth]
access_ttl_secs = 7200
refresh_ttl_secs = 3600
",
    );
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(file, "{toml}").unwrap();

    let error = Config::load(Some(file.path())).unwrap_err();

    assert!(matches!(error, ConfigError::InvalidAuthTtl));
}

#[test]
fn zero_token_ttl_is_rejected() {
    let error = load(
        "[enrollment]
token_ttl_secs = 0
",
    )
    .unwrap_err();

    assert!(matches!(error, ConfigError::InvalidEnrollment));
}

#[test]
fn loopback_bind_is_accepted() {
    let config = load("[http]\nbind = \"127.0.0.1:9000\"\n").unwrap();

    assert_eq!(config.http.bind.port(), 9000);
}

#[test]
fn plaintext_bind_outside_loopback_is_rejected() {
    let error = load("[http]\nbind = \"0.0.0.0:8080\"\n").unwrap_err();

    assert!(matches!(error, ConfigError::PlaintextOutsideLoopback(_)));
}

#[test]
fn heartbeat_interval_must_be_shorter_than_timeout() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    // Ajouté à la fin de la section `[daemon]`.
    write!(
        file,
        "{}\nheartbeat_interval_secs = 45\n",
        REQUIRED.trim_end()
    )
    .unwrap();

    let error = Config::load(Some(file.path())).unwrap_err();

    assert!(matches!(error, ConfigError::InvalidDaemonTimings));
}

#[test]
fn missing_database_section_is_an_error() {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    write!(
        file,
        "[daemon]\nca_cert = \"a\"\ncert = \"b\"\nkey = \"c\"\n"
    )
    .unwrap();

    let error = Config::load(Some(file.path())).unwrap_err();

    assert!(matches!(error, ConfigError::Load(_)));
}

#[test]
fn missing_config_file_is_an_error() {
    let error = Config::load(Some(std::path::Path::new("does/not/exist.toml"))).unwrap_err();

    assert!(matches!(error, ConfigError::Load(_)));
}
