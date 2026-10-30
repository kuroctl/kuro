//! Calcul des stats d'un serveur à partir des mesures Docker de ses conteneurs (ADR-0014).

use bollard::models::ContainerStatsResponse;

/// Mesure d'un conteneur en marche.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sample {
    /// 1000 = un cœur plein.
    pub cpu_millis: u64,
    /// Mémoire utilisée, hors cache de pages (inactif).
    pub memory_bytes: u64,
    pub memory_limit_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

impl Sample {
    /// `None` pour un conteneur arrêté : Docker ne fournit alors aucune mesure de mémoire.
    pub fn from_docker(stats: &ContainerStatsResponse) -> Option<Self> {
        let memory = stats.memory_stats.as_ref()?;
        let usage = memory.usage?;
        // cgroup v2 : `inactive_file` ; cgroup v1 : `total_inactive_file`.
        let inactive = memory
            .stats
            .as_ref()
            .and_then(|s| {
                s.get("inactive_file")
                    .or_else(|| s.get("total_inactive_file"))
            })
            .copied()
            .unwrap_or(0);
        let (rx, tx) = stats
            .networks
            .iter()
            .flat_map(|networks| networks.values())
            .fold((0_u64, 0_u64), |(rx, tx), n| {
                (
                    rx.saturating_add(n.rx_bytes.unwrap_or(0)),
                    tx.saturating_add(n.tx_bytes.unwrap_or(0)),
                )
            });
        Some(Self {
            cpu_millis: cpu_millis(stats),
            memory_bytes: usage.saturating_sub(inactive),
            memory_limit_bytes: memory.limit.unwrap_or(0),
            network_rx_bytes: rx,
            network_tx_bytes: tx,
        })
    }

    /// Somme des conteneurs d'un serveur.
    pub fn sum(samples: impl IntoIterator<Item = Self>) -> Self {
        samples.into_iter().fold(Self::default(), |total, s| Self {
            cpu_millis: total.cpu_millis.saturating_add(s.cpu_millis),
            memory_bytes: total.memory_bytes.saturating_add(s.memory_bytes),
            memory_limit_bytes: total
                .memory_limit_bytes
                .saturating_add(s.memory_limit_bytes),
            network_rx_bytes: total.network_rx_bytes.saturating_add(s.network_rx_bytes),
            network_tx_bytes: total.network_tx_bytes.saturating_add(s.network_tx_bytes),
        })
    }
}

/// Part du temps CPU de l'hôte consommée entre la mesure précédente et celle-ci, rapportée au
/// nombre de cœurs (formule de `docker stats`).
fn cpu_millis(stats: &ContainerStatsResponse) -> u64 {
    let (Some(now), Some(before)) = (&stats.cpu_stats, &stats.precpu_stats) else {
        return 0;
    };
    let total = |c: &bollard::models::ContainerCpuStats| {
        c.cpu_usage
            .as_ref()
            .and_then(|u| u.total_usage)
            .unwrap_or(0)
    };
    let cpu_delta = total(now).saturating_sub(total(before));
    let system_delta = now
        .system_cpu_usage
        .unwrap_or(0)
        .saturating_sub(before.system_cpu_usage.unwrap_or(0));
    if cpu_delta == 0 || system_delta == 0 {
        return 0;
    }
    let cores = u128::from(now.online_cpus.unwrap_or(1).max(1));
    let millis = u128::from(cpu_delta) * cores * 1000 / u128::from(system_delta);
    u64::try_from(millis).unwrap_or(u64::MAX)
}
