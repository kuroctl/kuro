//! Tests de fonctionnalités du daemon : un module par feature, un seul binaire de test.
// Code de test : les helpers hors `#[test]` peuvent paniquer comme les tests eux-mêmes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod ctl_config_check;
mod ctl_enroll;
mod servers;
#[cfg(target_os = "linux")]
mod sftp;
