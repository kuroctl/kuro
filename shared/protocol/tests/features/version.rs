use kuro_protocol::PROTOCOL_VERSION;
use kuro_protocol::version::is_supported;

#[test]
fn current_version_is_supported() {
    assert!(is_supported(PROTOCOL_VERSION));
}

#[test]
fn other_versions_are_not_supported() {
    assert!(!is_supported(PROTOCOL_VERSION + 1));
    assert!(!is_supported(0));
}
