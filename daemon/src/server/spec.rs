//! Règles du daemon sur une spécification reçue (ADR-0013). L'API valide déjà les prefabs ; le
//! daemon revérifie ce qui protège l'hôte, sans lui faire confiance.

use std::collections::HashSet;

use kuro_protocol::api::server::{LifecycleSpec, ReadySpec, ServerDeploy};

/// Ports publiés : jamais un port privilégié de l'hôte.
pub(crate) const MIN_HOST_PORT: u16 = 1024;
const MIN_MEMORY_BYTES: u64 = 16 * 1024 * 1024;

pub fn check(spec: &ServerDeploy) -> Result<(), String> {
    if spec.services.is_empty() {
        return Err("no service".to_owned());
    }
    let mut names = HashSet::new();
    let mut host_ports = HashSet::new();
    for service in &spec.services {
        let name = &service.name;
        if !is_name(name) || !names.insert(name.as_str()) {
            return Err(format!("invalid or duplicate service name `{name}`"));
        }
        if service.image.trim().is_empty() {
            return Err(format!("service {name}: empty image"));
        }
        check_user(&service.user).map_err(|e| format!("service {name}: {e}"))?;
        for (key, value) in &service.env {
            if !is_env_key(key) || value.contains(['\0', '\n']) {
                return Err(format!(
                    "service {name}: invalid environment variable `{key}`"
                ));
            }
        }
        if service.limits.memory_bytes < MIN_MEMORY_BYTES
            || service.limits.cpu_millis == 0
            || service.limits.pids == 0
        {
            return Err(format!("service {name}: limits are required"));
        }
        for volume in &service.volumes {
            if !is_name(&volume.name) || !is_container_path(&volume.path) {
                return Err(format!("service {name}: invalid volume `{}`", volume.name));
            }
        }
        for path in &service.writable {
            if !is_container_path(path) {
                return Err(format!("service {name}: invalid writable path `{path}`"));
            }
        }
        for port in &service.ports {
            if port.container_port == 0 {
                return Err(format!("service {name}: invalid port `{}`", port.name));
            }
            if let Some(published) = &port.published {
                if published.host_port < MIN_HOST_PORT {
                    return Err(format!(
                        "service {name}: host port {} is privileged",
                        published.host_port
                    ));
                }
                if published.host_ip.parse::<std::net::IpAddr>().is_err() {
                    return Err(format!("service {name}: invalid host ip"));
                }
                if !host_ports.insert((
                    published.host_ip.clone(),
                    published.host_port,
                    port.protocol,
                )) {
                    return Err(format!("host port {} published twice", published.host_port));
                }
            }
        }
    }
    for service in &spec.services {
        for dependency in &service.depends_on {
            if dependency == &service.name || !names.contains(dependency.as_str()) {
                return Err(format!(
                    "service {}: unknown dependency `{dependency}`",
                    service.name
                ));
            }
        }
    }
    if start_order(spec).is_none() {
        return Err("services depend on each other in a cycle".to_owned());
    }
    for file in &spec.files {
        let Some(service) = spec.services.iter().find(|s| s.name == file.service) else {
            return Err(format!("file {}: unknown service", file.path));
        };
        let inside = is_container_path(&file.path)
            && service.volumes.iter().any(|v| {
                let root = v.path.trim_end_matches('/');
                file.path.starts_with(&format!("{root}/"))
            });
        if !inside {
            return Err(format!(
                "file {} is not under a volume of {}",
                file.path, service.name
            ));
        }
    }
    check_lifecycle(spec, &spec.lifecycle)
}

fn check_lifecycle(spec: &ServerDeploy, lifecycle: &LifecycleSpec) -> Result<(), String> {
    let Some(service) = spec.services.iter().find(|s| s.name == lifecycle.service) else {
        return Err("lifecycle: unknown service".to_owned());
    };
    match &lifecycle.ready {
        Some(ReadySpec::Console(pattern)) if regex::Regex::new(pattern).is_err() => {
            Err("lifecycle: invalid ready pattern".to_owned())
        }
        Some(ReadySpec::Healthcheck) if service.healthcheck.is_none() => {
            Err("lifecycle: ready on healthcheck without healthcheck".to_owned())
        }
        _ if lifecycle.stop_timeout_secs == 0 || lifecycle.stop_timeout_secs > 600 => {
            Err("lifecycle: stop timeout must be 1-600 seconds".to_owned())
        }
        _ => Ok(()),
    }
}

/// `uid:gid` numériques, uid non nul : jamais root dans un conteneur de serveur.
/// Nom de variable d'environnement : lettres, chiffres et `_`, sans commencer par un chiffre.
/// `A=B` ou une clé vide donneraient une ligne `KEY=VALUE` que le conteneur lirait autrement.
fn is_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn check_user(user: &str) -> Result<(u32, u32), String> {
    let (uid, gid) = user.split_once(':').ok_or("user must be uid:gid")?;
    let uid: u32 = uid.parse().map_err(|_| "user must be numeric uid:gid")?;
    let gid: u32 = gid.parse().map_err(|_| "user must be numeric uid:gid")?;
    if uid == 0 {
        return Err("runs as root".to_owned());
    }
    Ok((uid, gid))
}

/// Ordre de démarrage respectant `depends_on` ; `None` en cas de cycle.
pub fn start_order(spec: &ServerDeploy) -> Option<Vec<&str>> {
    let mut order: Vec<&str> = Vec::with_capacity(spec.services.len());
    while order.len() < spec.services.len() {
        let next = spec.services.iter().find(|s| {
            !order.contains(&s.name.as_str())
                && s.depends_on.iter().all(|d| order.contains(&d.as_str()))
        })?;
        order.push(&next.name);
    }
    Some(order)
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && name.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_container_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains('\0') && !path.split('/').any(|p| p == "..")
}
