//! Historique des stats (ADR-0043) : une mesure toutes les dix secondes pour chaque serveur en
//! marche, regardé ou non, agrégée par minute et envoyée en `server.metrics`. Les minutes que le
//! lien n'a pas pu prendre sont gardées (une heure au plus) et renvoyées ensuite.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kuro_protocol::DaemonMessage;
use kuro_protocol::daemon::ServerMetrics;
use kuro_protocol::daemon::server::ServerState;
use tokio::sync::mpsc::error::TrySendError;
use uuid::Uuid;

use super::Manager;
use super::watch::collect;
use crate::docker::container_name;
use crate::stats::Sample;

/// Période des mesures hors spectateur.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(10);
const MINUTE_MS: i64 = 60_000;
/// Minutes gardées quand le lien ne suit pas : une heure.
const PENDING_MAX: usize = 60;

/// Minute en cours d'un serveur.
#[derive(Debug, Clone)]
struct Minute {
    from_ms: i64,
    samples: u32,
    cpu_sum: u64,
    cpu_max: u64,
    memory_sum: u64,
    memory_max: u64,
    memory_limit: u64,
    /// Compteurs cumulés à la première et à la dernière mesure.
    rx: (u64, u64),
    tx: (u64, u64),
}

impl Minute {
    fn start(from_ms: i64, sample: Sample) -> Self {
        Self {
            from_ms,
            samples: 1,
            cpu_sum: sample.cpu_millis,
            cpu_max: sample.cpu_millis,
            memory_sum: sample.memory_bytes,
            memory_max: sample.memory_bytes,
            memory_limit: sample.memory_limit_bytes,
            rx: (sample.network_rx_bytes, sample.network_rx_bytes),
            tx: (sample.network_tx_bytes, sample.network_tx_bytes),
        }
    }

    fn add(&mut self, sample: Sample) {
        self.samples += 1;
        self.cpu_sum = self.cpu_sum.saturating_add(sample.cpu_millis);
        self.cpu_max = self.cpu_max.max(sample.cpu_millis);
        self.memory_sum = self.memory_sum.saturating_add(sample.memory_bytes);
        self.memory_max = self.memory_max.max(sample.memory_bytes);
        self.memory_limit = sample.memory_limit_bytes;
        // Un conteneur redémarré repart de zéro : la différence serait négative, on repart aussi.
        if sample.network_rx_bytes < self.rx.1 {
            self.rx.0 = 0;
        }
        if sample.network_tx_bytes < self.tx.1 {
            self.tx.0 = 0;
        }
        self.rx.1 = sample.network_rx_bytes;
        self.tx.1 = sample.network_tx_bytes;
    }

    fn finish(&self, server_id: Uuid) -> ServerMetrics {
        let samples = u64::from(self.samples.max(1));
        ServerMetrics {
            server_id,
            from_ms: self.from_ms,
            to_ms: self.from_ms + MINUTE_MS,
            samples: self.samples,
            cpu_millis_avg: self.cpu_sum / samples,
            cpu_millis_max: self.cpu_max,
            memory_bytes_avg: self.memory_sum / samples,
            memory_bytes_max: self.memory_max,
            memory_limit_bytes: self.memory_limit,
            network_rx_bytes: self.rx.1.saturating_sub(self.rx.0),
            network_tx_bytes: self.tx.1.saturating_sub(self.tx.0),
        }
    }
}

/// Minutes en cours et minutes révolues en attente d'envoi.
#[derive(Debug, Default)]
pub struct MetricsState {
    current: Mutex<HashMap<Uuid, Minute>>,
    pending: Mutex<VecDeque<ServerMetrics>>,
}

impl MetricsState {
    fn current(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Minute>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, VecDeque<ServerMetrics>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ajoute une mesure prise à `now_ms` ; rend la minute révolue, le cas échéant.
    pub fn record(&self, server_id: Uuid, now_ms: i64, sample: Sample) -> Option<ServerMetrics> {
        let minute = now_ms - now_ms.rem_euclid(MINUTE_MS);
        let mut current = self.current();
        match current.get_mut(&server_id) {
            Some(open) if open.from_ms == minute => {
                open.add(sample);
                None
            }
            Some(open) => {
                let done = open.finish(server_id);
                *open = Minute::start(minute, sample);
                Some(done)
            }
            None => {
                current.insert(server_id, Minute::start(minute, sample));
                None
            }
        }
    }

    /// Minute en cours d'un serveur qui s'arrête ou disparaît : rendue telle quelle.
    pub fn close(&self, server_id: Uuid) -> Option<ServerMetrics> {
        self.current()
            .remove(&server_id)
            .map(|minute| minute.finish(server_id))
    }

    pub fn push(&self, metrics: ServerMetrics) {
        let mut pending = self.pending();
        if pending.len() >= PENDING_MAX {
            pending.pop_front();
        }
        pending.push_back(metrics);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

impl Manager {
    /// Mesure les serveurs en marche toutes les [`SAMPLE_EVERY`] et envoie chaque minute révolue.
    pub async fn collect_metrics(&self) {
        let mut ticks = tokio::time::interval(SAMPLE_EVERY);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            self.sample_metrics().await;
            self.flush_metrics();
        }
    }

    /// Une passe de mesures : les serveurs annoncés en marche, par leurs conteneurs.
    pub async fn sample_metrics(&self) {
        let running: Vec<Uuid> = self
            .inner
            .announced()
            .iter()
            .filter(|(_, (state, _))| *state == ServerState::Running)
            .map(|(id, _)| *id)
            .collect();
        // Les serveurs qui ne tournent plus livrent leur minute entamée.
        let stale: Vec<Uuid> = self
            .inner
            .metrics
            .current()
            .keys()
            .filter(|id| !running.contains(id))
            .copied()
            .collect();
        for server_id in stale {
            if let Some(done) = self.inner.metrics.close(server_id) {
                self.inner.metrics.push(done);
            }
        }
        for server_id in running {
            let Ok(Some(deploy)) = self.spec_opt(server_id) else {
                continue;
            };
            let names: Vec<String> = deploy
                .services
                .iter()
                .map(|s| container_name(server_id, &s.name))
                .collect();
            let measures = collect(&self.inner.docker, &names).await;
            if measures.is_empty() {
                continue;
            }
            let sample = Sample::sum(measures);
            if let Some(done) = self.inner.metrics.record(server_id, now_ms(), sample) {
                self.inner.metrics.push(done);
            }
        }
    }

    /// Envoie les minutes en attente tant que le lien les prend ; les autres attendent.
    pub fn flush_metrics(&self) {
        loop {
            let next = self.inner.metrics.pending().front().cloned();
            let Some(metrics) = next else {
                return;
            };
            match self
                .inner
                .streams
                .try_send(DaemonMessage::ServerMetrics(metrics))
            {
                Ok(()) => {
                    self.inner.metrics.pending().pop_front();
                }
                Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(cpu: u64, memory: u64, rx: u64) -> Sample {
        Sample {
            cpu_millis: cpu,
            memory_bytes: memory,
            memory_limit_bytes: 4096,
            network_rx_bytes: rx,
            network_tx_bytes: rx * 2,
        }
    }

    #[test]
    fn a_minute_aggregates_averages_peaks_and_network_deltas() {
        let state = MetricsState::default();
        let id = Uuid::nil();
        assert!(state.record(id, 60_000, sample(100, 1000, 500)).is_none());
        assert!(state.record(id, 70_000, sample(300, 3000, 800)).is_none());
        assert!(state.record(id, 119_000, sample(200, 2000, 1100)).is_none());
        // Minute suivante : la précédente est rendue.
        let done = state.record(id, 120_000, sample(50, 500, 1200)).unwrap();
        assert_eq!(done.from_ms, 60_000);
        assert_eq!(done.to_ms, 120_000);
        assert_eq!(done.samples, 3);
        assert_eq!(done.cpu_millis_avg, 200);
        assert_eq!(done.cpu_millis_max, 300);
        assert_eq!(done.memory_bytes_avg, 2000);
        assert_eq!(done.memory_bytes_max, 3000);
        assert_eq!(done.memory_limit_bytes, 4096);
        assert_eq!(done.network_rx_bytes, 600);
        assert_eq!(done.network_tx_bytes, 1200);
        // Un serveur arrêté livre sa minute entamée.
        let closed = state.close(id).unwrap();
        assert_eq!(closed.from_ms, 120_000);
        assert_eq!(closed.samples, 1);
        assert!(state.close(id).is_none());
    }

    #[test]
    fn a_restarted_container_does_not_yield_a_negative_delta_and_pending_is_bounded() {
        let state = MetricsState::default();
        let id = Uuid::nil();
        state.record(id, 0, sample(1, 1, 9_000));
        state.record(id, 10_000, sample(1, 1, 100));
        let done = state.record(id, 60_000, sample(1, 1, 0)).unwrap();
        assert_eq!(done.network_rx_bytes, 100);
        for i in 0..70 {
            state.push(ServerMetrics {
                from_ms: i,
                ..done.clone()
            });
        }
        let pending = state.pending();
        assert_eq!(pending.len(), PENDING_MAX);
        assert_eq!(pending.front().unwrap().from_ms, 10);
    }
}
