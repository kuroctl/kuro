//! Caractéristiques de l'hôte annoncées dans `hello`.

use std::env::consts;
use std::thread::available_parallelism;

use kuro_protocol::daemon::HostInfo;

pub(super) fn host_info() -> HostInfo {
    HostInfo {
        os: consts::OS.to_owned(),
        arch: consts::ARCH.to_owned(),
        cpu_cores: available_parallelism()
            .map(|cores| u32::try_from(cores.get()).unwrap_or(u32::MAX))
            .unwrap_or(1),
        memory_bytes: total_memory().unwrap_or(0),
    }
}

/// Mémoire totale lue dans `/proc/meminfo` (ligne `MemTotal: <n> kB`).
#[cfg(target_os = "linux")]
fn total_memory() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = meminfo.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

/// Le daemon ne cible que Linux ; ailleurs (poste de dev) la mémoire est inconnue.
#[cfg(not(target_os = "linux"))]
fn total_memory() -> Option<u64> {
    None
}
