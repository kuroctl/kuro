//! Compatibilité d'une spécification avec la version du daemon du nœud (ADR-0039).
//!
//! Un daemon antérieur répondrait `error` à une valeur qu'il ne connaît pas, mais seulement
//! après la création du serveur et de ses allocations. L'API connaît la version de chaque daemon
//! (`hello`) : elle refuse avant d'envoyer, avec `daemon_too_old`, et l'admin met le nœud à jour.
//! Une version illisible (build de développement) ne bloque rien : elle est journalisée.

use kuro_protocol::api::server::{FileContent, ServerDeploy, StopSpec};
use semver::Version;

use super::ServersError;
use crate::error_code::ErrorCode;

/// Fonctionnalité du protocole qu'un daemon doit connaître, et la version qui l'a apportée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonFeature {
    /// `SIGWINCH`, `SIGHUP` et le signal de l'image (ADR-0039 §5).
    StopSignals,
    /// Valeurs non textuelles dans `managed-keys` (ADR-0039 §4).
    TypedManagedKeys,
    /// Actions d'un prefab (ADR-0039 §3).
    Actions,
}

impl DaemonFeature {
    pub fn min_version(self) -> Version {
        match self {
            Self::StopSignals | Self::TypedManagedKeys | Self::Actions => Version::new(0, 2, 0),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::StopSignals => "stop_signals",
            Self::TypedManagedKeys => "typed_managed_keys",
            Self::Actions => "actions",
        }
    }
}

/// Ce qu'une spécification exige du daemon, au-delà de la base.
pub fn required_features(deploy: &ServerDeploy) -> Vec<DaemonFeature> {
    let mut features = Vec::new();
    let new_signal = match &deploy.lifecycle.stop {
        StopSpec::Image => true,
        StopSpec::Signal(signal) => !matches!(
            signal,
            kuro_protocol::api::server::StopSignal::Term
                | kuro_protocol::api::server::StopSignal::Int
                | kuro_protocol::api::server::StopSignal::Quit
        ),
        StopSpec::Console(_) => false,
    };
    if new_signal {
        features.push(DaemonFeature::StopSignals);
    }
    let typed_keys = deploy.files.iter().any(|file| match &file.content {
        FileContent::ManagedKeys { keys } => keys.values().any(|value| !value.is_string()),
        _ => false,
    });
    if typed_keys {
        features.push(DaemonFeature::TypedManagedKeys);
    }
    features
}

/// Refuse la spécification si le daemon du nœud est antérieur à une fonctionnalité qu'elle
/// exige. `None` ou illisible : accepté, journalisé.
pub fn check(
    node_name: &str,
    daemon_version: Option<&str>,
    deploy: &ServerDeploy,
) -> Result<(), ServersError> {
    check_features(node_name, daemon_version, &required_features(deploy))
}

/// Une seule fonctionnalité (action, par exemple).
pub fn check_feature(
    node_name: &str,
    daemon_version: Option<&str>,
    feature: DaemonFeature,
) -> Result<(), ServersError> {
    check_features(node_name, daemon_version, &[feature])
}

fn check_features(
    node_name: &str,
    daemon_version: Option<&str>,
    required: &[DaemonFeature],
) -> Result<(), ServersError> {
    if required.is_empty() {
        return Ok(());
    }
    let Some(version) = daemon_version.and_then(|v| Version::parse(v).ok()) else {
        tracing::warn!(node = node_name, version = ?daemon_version, "daemon version unknown, compatibility not checked");
        return Ok(());
    };
    for feature in required {
        if version < feature.min_version() {
            tracing::warn!(
                node = node_name,
                %version,
                feature = feature.as_str(),
                required = %feature.min_version(),
                "daemon too old for this prefab"
            );
            return Err(ServersError::Conflict(ErrorCode::DaemonTooOld));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kuro_protocol::api::server::{
        FileFormat, FileSpec, LifecycleSpec, Limits, ServiceSpec, StopSignal,
    };
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn deploy(stop: StopSpec) -> ServerDeploy {
        ServerDeploy {
            server_id: Uuid::now_v7(),
            services: vec![ServiceSpec {
                name: "app".to_owned(),
                image: "docker.io/library/alpine:3.20".to_owned(),
                command: Vec::new(),
                env: BTreeMap::new(),
                ports: Vec::new(),
                volumes: Vec::new(),
                writable: Vec::new(),
                user: "1000:1000".to_owned(),
                limits: Limits {
                    memory_bytes: 64 << 20,
                    cpu_millis: 500,
                    pids: 64,
                },
                depends_on: Vec::new(),
                healthcheck: None,
            }],
            files: Vec::new(),
            lifecycle: LifecycleSpec {
                service: "app".to_owned(),
                ready: None,
                stop,
                stop_timeout_secs: 10,
            },
            install: None,
            disk_bytes: 0,
            suspended: false,
            defer: false,
            web: Vec::new(),
        }
    }

    #[test]
    fn classic_specs_need_nothing_and_pass_any_daemon() {
        let classic = deploy(StopSpec::Signal(StopSignal::Term));
        assert!(required_features(&classic).is_empty());
        assert!(check("node-1", Some("0.1.0"), &classic).is_ok());
        assert!(check("node-1", None, &classic).is_ok());
    }

    #[test]
    fn new_signals_and_typed_keys_need_a_recent_daemon() {
        let winch = deploy(StopSpec::Signal(StopSignal::Winch));
        assert_eq!(required_features(&winch), vec![DaemonFeature::StopSignals]);
        assert!(matches!(
            check("node-1", Some("0.1.0"), &winch),
            Err(ServersError::Conflict(ErrorCode::DaemonTooOld))
        ));
        assert!(check("node-1", Some("0.2.0"), &winch).is_ok());
        assert!(check("node-1", Some("1.0.0"), &winch).is_ok());
        // Build de développement illisible : pas de blocage.
        assert!(check("node-1", Some("dev"), &winch).is_ok());

        let mut typed = deploy(StopSpec::Image);
        typed.files.push(FileSpec {
            service: "app".to_owned(),
            path: "/data/app.json".to_owned(),
            format: FileFormat::Json,
            content: FileContent::ManagedKeys {
                keys: BTreeMap::from([("port".to_owned(), json!(8080))]),
            },
        });
        assert_eq!(
            required_features(&typed),
            vec![DaemonFeature::StopSignals, DaemonFeature::TypedManagedKeys]
        );
        assert!(matches!(
            check("node-1", Some("0.1.9"), &typed),
            Err(ServersError::Conflict(ErrorCode::DaemonTooOld))
        ));
    }
}
