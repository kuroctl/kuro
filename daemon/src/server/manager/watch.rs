//! Serveurs regardés (ADR-0014) : console et stats lues seulement pendant un `server.watch`, et
//! commandes console.

use std::time::Duration;

use crate::fs::FsError;
use futures_util::StreamExt;
use kuro_protocol::DaemonMessage;
use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::daemon::{ConsoleLine, ConsoleOutput, ServerStats};
use kuro_protocol::limits::{
    CONSOLE_HISTORY_LINES, MAX_CONSOLE_BATCH_LINES, is_valid_command, truncate_console_line,
};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::disk::ensure_active;
use super::{CommandError, Manager};
use crate::docker::{ConsoleFrom, Docker, TimedLine, container_name};
use crate::stats::Sample;

/// Regroupement des lignes de console avant envoi.
const BATCH_DELAY: Duration = Duration::from_millis(100);
/// Période des stats.
const STATS_PERIOD: Duration = Duration::from_secs(2);
/// Attente avant de relire la console d'un conteneur arrêté.
const CONSOLE_RETRY: Duration = Duration::from_secs(1);

impl Manager {
    /// Commence ou cesse de diffuser la console et les stats d'un serveur. Un `watch` déjà actif
    /// est conservé : l'historique n'est pas renvoyé deux fois.
    pub fn watch(&self, server_id: Uuid, watch: bool) -> Result<(), CommandError> {
        if !watch {
            self.unwatch(server_id);
            return Ok(());
        }
        let deploy = self.spec(server_id)?;
        let token = {
            let mut watches = self.inner.watches();
            if watches.contains_key(&server_id) {
                return Ok(());
            }
            let token = CancellationToken::new();
            watches.insert(server_id, token.clone());
            token
        };
        tracing::debug!(%server_id, "watch started");
        let this = self.clone();
        let console_token = token.clone();
        let console_deploy = deploy.clone();
        tokio::spawn(async move {
            this.stream_console(&console_deploy, &console_token).await;
        });
        let this = self.clone();
        tokio::spawn(async move {
            this.stream_stats(&deploy, &token).await;
        });
        Ok(())
    }

    pub(super) fn unwatch(&self, server_id: Uuid) {
        if let Some(token) = self.inner.watches().remove(&server_id) {
            tracing::debug!(%server_id, "watch stopped");
            token.cancel();
        }
    }

    /// Le lien est tombé : l'API renverra ses `server.watch` à la reconnexion.
    pub fn unwatch_all(&self) {
        for (_, token) in self.inner.watches().drain() {
            token.cancel();
        }
    }

    /// Écrit une ligne sur l'entrée du service principal, s'il tourne.
    pub async fn command(&self, server_id: Uuid, line: &str) -> Result<(), CommandError> {
        if !is_valid_command(line) {
            return Err(CommandError::InvalidSpec(
                "invalid console command".to_owned(),
            ));
        }
        let deploy = self.spec(server_id)?;
        ensure_active(&deploy)?;
        let main = container_name(server_id, &deploy.lifecycle.service);
        let docker = &self.inner.docker;
        let running = docker
            .status(&main)
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))?
            .is_some_and(|status| status.running);
        if !running {
            return Err(CommandError::NotRunning);
        }
        docker
            .send_line(&main, line)
            .await
            .map_err(|e| CommandError::Internal(e.to_string()))
    }

    pub(super) fn spec(&self, server_id: Uuid) -> Result<ServerDeploy, CommandError> {
        self.spec_opt(server_id)
            .map_err(|e| CommandError::Internal(e.to_string()))?
            .ok_or(CommandError::UnknownServer)
    }

    /// Spécification déployée, depuis le cache ; lue sur le disque la première fois seulement.
    pub(super) fn spec_opt(&self, server_id: Uuid) -> Result<Option<ServerDeploy>, FsError> {
        if let Some(deploy) = self.inner.specs().get(&server_id) {
            return Ok(Some(deploy.clone()));
        }
        let loaded = self.dir(server_id).load_spec()?;
        if let Some(deploy) = &loaded {
            self.inner.specs().insert(server_id, deploy.clone());
        }
        Ok(loaded)
    }

    /// Écrit la spécification (atomiquement) et met le cache à jour.
    pub(super) fn store_spec(&self, deploy: &ServerDeploy) -> Result<(), FsError> {
        self.dir(deploy.server_id).save_spec(deploy)?;
        self.inner.specs().insert(deploy.server_id, deploy.clone());
        Ok(())
    }

    /// Historique puis direct ; relit la console à chaque redémarrage du conteneur, sans
    /// renvoyer les lignes déjà transmises.
    async fn stream_console(&self, deploy: &ServerDeploy, token: &CancellationToken) {
        let server_id = deploy.server_id;
        let main = container_name(server_id, &deploy.lifecycle.service);
        let mut batch = Batch::new(server_id, self.inner.streams.clone());
        let mut from = ConsoleFrom::Tail(CONSOLE_HISTORY_LINES);
        let mut last_ns = i128::MIN;
        loop {
            let mut lines = Box::pin(self.inner.docker.console_lines(&main, from));
            let mut flush = tokio::time::interval(BATCH_DELAY);
            loop {
                tokio::select! {
                    () = token.cancelled() => return,
                    _ = flush.tick() => {
                        if !batch.flush() {
                            return;
                        }
                    }
                    line = lines.next() => match line {
                        Some(line) if line.at_ns == 0 || line.at_ns > last_ns => {
                            last_ns = last_ns.max(line.at_ns);
                            if !batch.push(line) {
                                return;
                            }
                        }
                        Some(_) => {}
                        None => break,
                    },
                }
            }
            if !batch.flush() {
                return;
            }
            // Conteneur arrêté (ou absent) : on attend son redémarrage.
            from = ConsoleFrom::Since(seconds(last_ns));
            tokio::select! {
                () = token.cancelled() => return,
                () = tokio::time::sleep(CONSOLE_RETRY) => {}
            }
        }
    }

    /// Une mesure toutes les [`STATS_PERIOD`], somme des conteneurs en marche. Rien n'est envoyé
    /// pour un serveur arrêté.
    async fn stream_stats(&self, deploy: &ServerDeploy, token: &CancellationToken) {
        let server_id = deploy.server_id;
        let names: Vec<String> = deploy
            .services
            .iter()
            .map(|s| container_name(server_id, &s.name))
            .collect();
        let mut period = tokio::time::interval(STATS_PERIOD);
        loop {
            tokio::select! {
                () = token.cancelled() => return,
                _ = period.tick() => {}
            }
            let measures = tokio::select! {
                () = token.cancelled() => return,
                measures = collect(&self.inner.docker, &names) => measures,
            };
            if measures.is_empty() {
                continue;
            }
            let total = Sample::sum(measures);
            let stats = DaemonMessage::ServerStats(ServerStats {
                server_id,
                cpu_millis: total.cpu_millis,
                memory_bytes: total.memory_bytes,
                memory_limit_bytes: total.memory_limit_bytes,
                network_rx_bytes: total.network_rx_bytes,
                network_tx_bytes: total.network_tx_bytes,
            });
            match self.inner.streams.try_send(stats) {
                // Une mesure perdue est remplacée par la suivante.
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Closed(_)) => return,
            }
        }
    }
}

async fn collect(docker: &Docker, names: &[String]) -> Vec<Sample> {
    let measures =
        futures_util::future::join_all(names.iter().map(|name| docker.stats(name))).await;
    measures
        .iter()
        .flatten()
        .filter_map(Sample::from_docker)
        .collect()
}

fn seconds(ns: i128) -> i64 {
    if ns <= 0 {
        return 0;
    }
    i64::try_from(ns / 1_000_000_000).unwrap_or(0)
}

/// Lignes en attente d'envoi. Si le lien ne suit pas, le lot est jeté et compté dans `dropped`.
struct Batch {
    server_id: Uuid,
    sender: mpsc::Sender<DaemonMessage>,
    lines: Vec<ConsoleLine>,
    dropped: u64,
}

impl Batch {
    fn new(server_id: Uuid, sender: mpsc::Sender<DaemonMessage>) -> Self {
        Self {
            server_id,
            sender,
            lines: Vec::new(),
            dropped: 0,
        }
    }

    /// `false` si le lien est fermé.
    fn push(&mut self, line: TimedLine) -> bool {
        self.lines.push(ConsoleLine {
            at_ms: line.at_ms(),
            text: truncate_console_line(line.text),
        });
        self.lines.len() < MAX_CONSOLE_BATCH_LINES || self.flush()
    }

    /// `false` si le lien est fermé.
    fn flush(&mut self) -> bool {
        if self.lines.is_empty() && self.dropped == 0 {
            return true;
        }
        let lines = std::mem::take(&mut self.lines);
        let count = lines.len() as u64;
        let output = DaemonMessage::ConsoleLine(ConsoleOutput {
            server_id: self.server_id,
            lines,
            dropped: self.dropped,
        });
        match self.sender.try_send(output) {
            Ok(()) => {
                self.dropped = 0;
                true
            }
            Err(TrySendError::Full(_)) => {
                self.dropped = self.dropped.saturating_add(count);
                true
            }
            Err(TrySendError::Closed(_)) => false,
        }
    }
}
