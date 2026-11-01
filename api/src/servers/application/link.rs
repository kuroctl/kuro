//! Événements du lien daemon : serveurs assignés et états rapportés.

use std::sync::{Arc, OnceLock};

use futures_util::future::BoxFuture;
use kuro_protocol::api::server::PowerAction;
use kuro_protocol::daemon::server::{ServerState as Reported, ServerStateReport};
use kuro_protocol::daemon::{
    BackupDone, ConsoleOutput, ContentDone, ServerDisk, ServerMetrics, ServerStats,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::manage::send_power;
use super::{ConsoleHub, ServersState};
use crate::audit::domain::AuditContext;
use crate::nodes::domain::NodeId;
use crate::notifications::application as notifications;
use crate::notifications::domain::{
    DISK_HIGH_RATIO, DISK_RESET_RATIO, Event, EventKind, ServerRef,
};
use crate::servers::domain::{
    NodeCommands, ServerEvent, ServerEvents, ServerState, install_outcome,
};
use crate::servers::infra::{BackupRepository, ServerRepository};

/// Longueur maximale d'une raison rapportée par un daemon (stockée et affichée).
const MAX_REASON_CHARS: usize = 500;

#[derive(Debug, Clone)]
pub struct LinkEvents {
    pool: PgPool,
    console: ConsoleHub,
    /// Démarrage une fois installé, envoyé par l'API elle-même.
    commands: Arc<dyn NodeCommands>,
    /// Cas d'usage des serveurs, rattachés une fois construits (ils ont besoin du registre des
    /// liens, qui a besoin de ces événements) : redéploiement après des paquets extraits.
    servers: Arc<OnceLock<ServersState>>,
    /// Réconciliation DNS (ADR-0026), réveillée quand un nœud confirme une suppression : ses
    /// enregistrements partent sans attendre la passe suivante.
    dns: Arc<OnceLock<crate::dns::application::DnsState>>,
}

impl LinkEvents {
    pub fn new(pool: PgPool, console: ConsoleHub, commands: Arc<dyn NodeCommands>) -> Self {
        Self {
            pool,
            console,
            commands,
            servers: Arc::new(OnceLock::new()),
            dns: Arc::new(OnceLock::new()),
        }
    }

    /// Rattache les cas d'usage des serveurs ; un second rattachement est ignoré.
    pub fn attach(&self, servers: ServersState) {
        let _ = self.servers.set(servers);
    }

    /// Rattache la réconciliation DNS ; un second rattachement est ignoré.
    pub fn attach_dns(&self, dns: crate::dns::application::DnsState) {
        let _ = self.dns.set(dns);
    }

    async fn record(&self, node: NodeId, report: ServerStateReport) -> Result<(), sqlx::Error> {
        let mut conn = self.pool.acquire().await?;
        let node_id = node.as_uuid();
        let owned = match ServerState::from_reported(report.state) {
            None => {
                let deleted =
                    ServerRepository::delete_if_owned(&mut conn, report.server_id, node_id).await?;
                if let Some(owner_ref) = &deleted {
                    self.emit(ServerEvent::Deleted {
                        server_id: report.server_id,
                        owner_ref: owner_ref.clone(),
                    });
                }
                deleted.is_some()
            }
            Some(state) => {
                let reason = report.reason.map(|r| {
                    r.chars()
                        .filter(|c| !c.is_control())
                        .take(MAX_REASON_CHARS)
                        .collect::<String>()
                });
                let previous = ServerRepository::set_state_if_owned(
                    &mut conn,
                    report.server_id,
                    node_id,
                    state.as_str(),
                    reason.as_deref(),
                )
                .await?;
                // Fin d'installation (ADR-0051) : prêt ou en échec, depuis `deploying`/`installing`.
                if let Some(installed) = previous
                    .as_deref()
                    .and_then(|previous| install_outcome(previous, state.as_str()))
                {
                    let owner_ref = ServerRepository::get(&mut conn, report.server_id)
                        .await?
                        .and_then(|server| server.owner_ref);
                    self.emit(if installed {
                        ServerEvent::Installed {
                            server_id: report.server_id,
                            owner_ref,
                        }
                    } else {
                        ServerEvent::InstallFailed {
                            server_id: report.server_id,
                            owner_ref,
                            reason,
                        }
                    });
                }
                previous.is_some()
            }
        };
        // Suppression confirmée : ses enregistrements DNS n'ont plus de raison d'attendre.
        if owned
            && report.state == Reported::Deleted
            && let Some(dns) = self.dns.get()
        {
            dns.wake();
        }
        // Arrêt inattendu : notifié (ADR-0044), avec la raison rapportée par le nœud.
        if owned
            && report.state == Reported::Failed
            && let Some(server) = ServerRepository::get(&mut conn, report.server_id).await?
        {
            let event = Event::now(EventKind::ServerCrashed)
                .server(server_ref(&server))
                .reason(server.state_reason.clone().unwrap_or_default());
            notifications::emit_detached(&self.pool, event);
        }
        // Le serveur démarre : ses changements de contenu en attente sont appliqués.
        if owned && report.state == Reported::Starting {
            ServerRepository::clear_pending(&mut conn, report.server_id).await?;
        }
        // Site web (ADR-0038) : état du proxy et du certificat, tel que le nœud le voit.
        if owned && let Some(web) = &report.web {
            let status = serde_json::to_value(web).unwrap_or(serde_json::Value::Null);
            ServerRepository::set_web_status_if_owned(
                &mut conn,
                report.server_id,
                node_id,
                &status,
            )
            .await?;
        }
        if !owned && report.state != Reported::Deleted {
            // Serveur d'un autre nœud, inconnu, ou en suppression : un daemon n'agit que sur ses
            // serveurs.
            tracing::warn!(%node_id, server_id = %report.server_id, state = ?report.state, "ignored state report");
            return Ok(());
        }
        // Fin du premier déploiement : démarrer si demandé à la création ; un échec annule la
        // demande, pour qu'une réinstallation ultérieure ne démarre pas le serveur par surprise.
        if matches!(report.state, Reported::Offline | Reported::Failed)
            && ServerRepository::take_start_when_installed(&mut conn, report.server_id, node_id)
                .await?
            && report.state == Reported::Offline
        {
            self.start(report.server_id);
        }
        Ok(())
    }

    /// Événement pour les extensions (ADR-0051), par les cas d'usage rattachés.
    fn emit(&self, event: ServerEvent) {
        if let Some(servers) = self.servers.get() {
            servers.events.emit(event);
        }
    }

    /// Démarrage audité au nom du système, hors de la boucle du lien : la commande attend la
    /// réponse du même daemon.
    fn start(&self, server_id: Uuid) {
        let (pool, commands) = (self.pool.clone(), self.commands.clone());
        tokio::spawn(async move {
            let context = AuditContext::system();
            if let Err(error) =
                send_power(&pool, &*commands, &context, server_id, PowerAction::Start).await
            {
                tracing::warn!(%server_id, %error, "failed to start the installed server");
            }
        });
    }
}

impl ServerEvents for LinkEvents {
    fn assigned(&self, node: NodeId) -> BoxFuture<'_, Result<Vec<Uuid>, sqlx::Error>> {
        Box::pin(async move {
            let mut conn = self.pool.acquire().await?;
            ServerRepository::ids_of_node(&mut conn, node.as_uuid()).await
        })
    }

    fn record_state(&self, node: NodeId, report: ServerStateReport) -> BoxFuture<'_, ()> {
        self.console.publish_state(node, &report);
        Box::pin(async move {
            if let Err(error) = self.record(node, report).await {
                tracing::error!(%error, "failed to record server state");
            }
        })
    }

    fn watched(&self, node: NodeId) -> Vec<Uuid> {
        self.console.watched_on(node)
    }

    fn console(&self, node: NodeId, output: ConsoleOutput) {
        self.console.publish_lines(node, output);
    }

    fn stats(&self, node: NodeId, stats: ServerStats) {
        self.console.publish_stats(node, stats);
    }

    fn metrics(&self, node: NodeId, metrics: ServerMetrics) {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            if let Err(error) = super::metrics::record(&pool, node, metrics).await {
                tracing::error!(%error, "failed to record server metrics");
            }
        });
    }

    // Hors de la boucle du lien : la rotation et le rattrapage envoient des commandes au nœud.
    fn disk_usage(&self, node: NodeId, disk: ServerDisk) {
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let used = i64::try_from(disk.used_bytes).unwrap_or(i64::MAX);
            let outcome = async {
                let mut conn = pool.acquire().await?;
                let before = ServerRepository::get(&mut conn, disk.server_id).await?;
                ServerRepository::record_disk_if_owned(
                    &mut conn,
                    disk.server_id,
                    node.as_uuid(),
                    used,
                )
                .await?;
                Ok::<_, sqlx::Error>(before)
            };
            match outcome.await {
                Err(error) => tracing::error!(%error, "failed to record a disk usage report"),
                // Quota franchi à la hausse (ADR-0044) : une fois, puis de nouveau après un
                // retour sous le seuil bas.
                Ok(Some(server)) if server.node_id == node.as_uuid() && server.disk_mb > 0 => {
                    let quota = i64::from(server.disk_mb) * 1024 * 1024;
                    let ratio = |bytes: i64| bytes as f64 / quota as f64;
                    let previous = server.disk_used_bytes.map_or(0.0, ratio);
                    let now = ratio(used);
                    if now >= DISK_HIGH_RATIO && previous < DISK_HIGH_RATIO
                        || now >= DISK_HIGH_RATIO && previous < DISK_RESET_RATIO
                    {
                        let event = Event::now(EventKind::ServerDiskHigh)
                            .server(server_ref(&server))
                            .details(serde_json::json!({
                                "percent": (now * 100.0).round() as u64,
                                "used_bytes": used,
                                "quota_bytes": quota,
                            }));
                        notifications::emit_detached(&pool, event);
                    }
                }
                Ok(_) => {}
            }
        });
    }

    fn content_done(&self, node: NodeId, done: ContentDone) {
        let (pool, commands) = (self.pool.clone(), self.commands.clone());
        let servers = self.servers.clone();
        tokio::spawn(async move {
            match super::content::record_done(&pool, &*commands, node, done).await {
                // Paquets inscrits : le serveur reçoit ses fichiers d'inscription à jour.
                Ok(Some(server_id)) => match servers.get() {
                    Some(state) => super::content::redeploy_registered(state, server_id).await,
                    None => tracing::warn!(%server_id, "packages recorded, server not redeployed"),
                },
                Ok(None) => {}
                Err(error) => tracing::error!(%error, "failed to record a content report"),
            }
        });
    }

    fn backup_done(&self, node: NodeId, done: BackupDone) {
        let (pool, commands) = (self.pool.clone(), self.commands.clone());
        tokio::spawn(async move {
            if let Err(error) = super::backups::record_done(&pool, &*commands, node, done).await {
                tracing::error!(%error, "failed to record a backup report");
            }
        });
    }

    fn running_backups(
        &self,
        node: NodeId,
    ) -> BoxFuture<'_, Result<Vec<(Uuid, Uuid)>, sqlx::Error>> {
        Box::pin(async move {
            let mut conn = self.pool.acquire().await?;
            BackupRepository::creating_on_node(&mut conn, node.as_uuid()).await
        })
    }

    // Hors de la boucle du lien, qui doit relayer les réponses du nœud.
    fn reconcile_backups(&self, node: NodeId, running: Vec<(Uuid, Uuid)>) {
        if running.is_empty() {
            return;
        }
        let (pool, commands) = (self.pool.clone(), self.commands.clone());
        tokio::spawn(async move {
            if let Err(error) = super::backups::reconcile(&pool, &*commands, node, running).await {
                tracing::warn!(%error, "failed to reconcile backups");
            }
        });
    }
}

/// Référence d'un serveur pour une notification (ADR-0044).
pub(crate) fn server_ref(server: &crate::servers::infra::ServerRecord) -> ServerRef {
    ServerRef {
        id: server.id,
        name: server.name.clone(),
        prefab_id: server.prefab_id.clone(),
        node_id: server.node_id,
    }
}
