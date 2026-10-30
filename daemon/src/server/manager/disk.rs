//! Espace disque des serveurs, quota et suspension (ADR-0019).
//!
//! L'espace d'un serveur : ses volumes, parcourus par l'accès sûr de l'ADR-0015 (un lien n'est
//! jamais suivi), et ses sauvegardes. Mesuré à intervalle régulier (`servers.
//! disk_measure_interval_secs`, 60 s par défaut) et après chaque opération qui écrit ; le dernier
//! résultat sert aux refus et part à l'API (`server.disk`).

use std::path::Path;
use std::time::Duration;

use kuro_protocol::DaemonMessage;
use kuro_protocol::api::server::{ServerDeploy, ServerLimits};
use kuro_protocol::daemon::FileKind;
use kuro_protocol::daemon::server::ServerDisk;
use uuid::Uuid;

use super::{CommandError, Manager, spec};

/// Délai de regroupement des mesures déclenchées par des écritures rapprochées.
const MEASURE_DEBOUNCE: Duration = Duration::from_secs(2);
use crate::docker::container_name;
use crate::fs::volumes::{VolumeError, Volumes};

impl Manager {
    /// Mesure tous les serveurs du nœud, toutes les `every`.
    pub async fn measure_disks(&self, every: Duration) {
        let mut ticks = tokio::time::interval(every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticks.tick().await;
            // Même cadence pour oublier les envois de fichiers abandonnés.
            self.sweep_uploads();
            for server_id in self.deployed() {
                self.measure(server_id).await;
            }
        }
    }

    /// Mesure le serveur, garde le résultat et le rapporte ; `None` si la mesure échoue.
    pub(super) async fn measure(&self, server_id: Uuid) -> Option<u64> {
        let root = self.dir(server_id).root().to_path_buf();
        // Deux parcours de disque à la fois au plus, quel que soit le nombre de serveurs.
        let _slot = self.inner.measure_slots.clone().acquire_owned().await;
        let used = match tokio::task::spawn_blocking(move || used_bytes(&root)).await {
            Ok(Ok(used)) => used,
            Ok(Err(error)) => {
                tracing::warn!(%server_id, %error, "disk measurement failed");
                return None;
            }
            Err(error) => {
                tracing::warn!(%server_id, %error, "disk measurement failed");
                return None;
            }
        };
        self.inner.disk().insert(server_id, used);
        let report = DaemonMessage::ServerDisk(ServerDisk {
            server_id,
            used_bytes: used,
        });
        if self.inner.streams.try_send(report).is_err() {
            // Perdu : la mesure suivante le remplacera.
            tracing::debug!(%server_id, "disk report dropped");
        }
        Some(used)
    }

    /// Mesure en tâche de fond, après une opération qui a écrit. Des écritures rapprochées
    /// (morceaux d'un envoi, fichiers d'une installation) ne donnent qu'une mesure.
    pub(super) fn measure_later(&self, server_id: Uuid) {
        if !self.inner.measuring().insert(server_id) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(MEASURE_DEBOUNCE).await;
            this.inner.measuring().remove(&server_id);
            this.measure(server_id).await;
        });
    }

    /// Au démarrage : les temporaires d'opérations interrompues (fichiers en cours d'écriture,
    /// dossiers d'extraction, archives de sauvegarde inachevées) disparaissent.
    pub async fn clean_up_at_boot(&self) {
        for server_id in self.deployed() {
            let root = self.dir(server_id).root().to_path_buf();
            let swept = tokio::task::spawn_blocking(move || {
                let temporaries = sweep_temporaries(&root).map_err(|e| e.to_string())?;
                crate::backup::Store::new(&root)
                    .clean(None)
                    .map_err(|e| e.to_string())?;
                Ok::<usize, String>(temporaries)
            })
            .await;
            match swept {
                Ok(Ok(0)) => {}
                Ok(Ok(count)) => tracing::info!(%server_id, count, "temporary files removed"),
                Ok(Err(error)) => tracing::warn!(%server_id, %error, "temporary files not removed"),
                Err(error) => tracing::warn!(%server_id, %error, "temporary files not removed"),
            }
        }
    }

    /// Refuse au-delà du quota, `extra` octets à écrire compris. Sans mesure connue, mesure
    /// d'abord.
    pub(super) async fn check_quota(
        &self,
        deploy: &ServerDeploy,
        extra: u64,
    ) -> Result<(), CommandError> {
        if deploy.disk_bytes == 0 {
            return Ok(());
        }
        let known = self.inner.disk().get(&deploy.server_id).copied();
        let used = match known {
            Some(used) => used,
            None => self.measure(deploy.server_id).await.unwrap_or(0),
        };
        // Au quota exactement, plus rien : même une opération sans taille connue est refusée.
        if used.saturating_add(extra.max(1)) > deploy.disk_bytes {
            return Err(CommandError::DiskQuotaExceeded);
        }
        Ok(())
    }

    /// Nouvelles limites (ADR-0019) : gardées dans la spécification, et appliquées aux
    /// conteneurs existants sans redémarrage.
    pub fn limits(&self, limits: ServerLimits) -> Result<(), CommandError> {
        let mut deploy = self.spec(limits.server_id)?;
        for service in &mut deploy.services {
            let new = limits.services.get(&service.name).ok_or_else(|| {
                CommandError::InvalidSpec(format!("no limits for service {}", service.name))
            })?;
            service.limits = *new;
        }
        deploy.disk_bytes = limits.disk_bytes;
        spec::check(&deploy).map_err(CommandError::InvalidSpec)?;
        let guard = self.acquire_power(limits.server_id)?;
        self.store_spec(&deploy)
            .map_err(|e| CommandError::Internal(e.to_string()))?;
        let this = self.clone();
        tokio::spawn(async move {
            for service in &deploy.services {
                let name = container_name(deploy.server_id, &service.name);
                if let Err(error) = this
                    .inner
                    .docker
                    .update_limits(&name, &service.limits)
                    .await
                {
                    // Les limites gardées s'appliqueront à la prochaine création du conteneur.
                    tracing::warn!(server_id = %deploy.server_id, %error, "live limits update failed");
                }
            }
            drop(guard);
        });
        Ok(())
    }

    /// Suspend (arrête le serveur) ou réactive. Gardé dans la spécification : un redémarrage
    /// du daemon ne lève pas la suspension.
    pub fn suspend(&self, server_id: Uuid, suspended: bool) -> Result<(), CommandError> {
        let mut deploy = self.spec(server_id)?;
        let guard = self.acquire_power(server_id)?;
        deploy.suspended = suspended;
        self.store_spec(&deploy)
            .map_err(|e| CommandError::Internal(e.to_string()))?;
        if !suspended {
            return Ok(());
        }
        let this = self.clone();
        tokio::spawn(async move {
            let main = container_name(server_id, &deploy.lifecycle.service);
            let running =
                matches!(this.inner.docker.status(&main).await, Ok(Some(status)) if status.running);
            if running && let Err(error) = this.stop_all(&deploy).await {
                tracing::warn!(%server_id, %error, "stop on suspension failed");
            }
            drop(guard);
        });
        Ok(())
    }

    /// Serveurs déployés sur ce nœud (une spécification gardée).
    pub(super) fn deployed(&self) -> Vec<Uuid> {
        let Ok(entries) = std::fs::read_dir(&self.inner.data_root) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<Uuid>().ok())
            .filter(|id| self.dir(*id).exists())
            .collect()
    }
}

/// Refus d'un serveur suspendu.
pub(super) fn ensure_active(deploy: &ServerDeploy) -> Result<(), CommandError> {
    if deploy.suspended {
        return Err(CommandError::Suspended);
    }
    Ok(())
}

/// Entrées temporaires de Kuro dans les volumes : écritures et envois interrompus, dossiers
/// d'extraction et archives de modpack abandonnés. Jamais celles du jeu.
fn is_temporary(name: &str) -> bool {
    name.starts_with(".kuro-tmp-")
        || name.starts_with(".kuro-extract-")
        || name.starts_with(".kuro-pack-")
}

/// Supprime les entrées temporaires des volumes du serveur ; retourne leur nombre.
fn sweep_temporaries(server_root: &Path) -> Result<usize, VolumeError> {
    let volumes = Volumes::new(server_root);
    let mut removed = 0;
    let mut pending: Vec<Vec<String>> = Vec::new();
    let volumes_root = server_root.join("volumes");
    for service in read_names(&volumes_root) {
        for volume in read_names(&volumes_root.join(&service)) {
            pending.push(vec![service.clone(), volume]);
        }
    }
    while let Some(dir) = pending.pop() {
        let segments: Vec<&str> = dir.iter().map(String::as_str).collect();
        let entries = match volumes.list(&segments) {
            Ok(entries) => entries,
            Err(VolumeError::NotFound | VolumeError::InvalidPath) => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let mut path = dir.clone();
            path.push(entry.name.clone());
            if is_temporary(&entry.name) {
                let segments: Vec<&str> = path.iter().map(String::as_str).collect();
                match volumes.delete(&segments) {
                    Ok(()) => removed += 1,
                    Err(VolumeError::NotFound) => {}
                    Err(error) => return Err(error),
                }
            } else if entry.kind == FileKind::Directory {
                pending.push(path);
            }
        }
    }
    Ok(removed)
}

/// Volumes (sans suivre de lien) et sauvegardes du serveur.
fn used_bytes(server_root: &Path) -> Result<u64, VolumeError> {
    let volumes = Volumes::new(server_root);
    let mut total = 0;
    let mut pending: Vec<Vec<String>> = Vec::new();
    let volumes_root = server_root.join("volumes");
    for service in read_names(&volumes_root) {
        for volume in read_names(&volumes_root.join(&service)) {
            pending.push(vec![service.clone(), volume]);
        }
    }
    while let Some(dir) = pending.pop() {
        let segments: Vec<&str> = dir.iter().map(String::as_str).collect();
        let entries = match volumes.list(&segments) {
            Ok(entries) => entries,
            // Supprimé ou remplacé par un lien pendant le parcours.
            Err(VolumeError::NotFound | VolumeError::InvalidPath) => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            match entry.kind {
                FileKind::File => total += entry.size,
                FileKind::Directory => {
                    let mut path = dir.clone();
                    path.push(entry.name);
                    pending.push(path);
                }
                FileKind::Symlink | FileKind::Other => {}
            }
        }
    }
    // Sauvegardes : dossier du daemon, hors d'atteinte des conteneurs.
    if let Ok(backups) = std::fs::read_dir(server_root.join("backups")) {
        for entry in backups.filter_map(Result::ok) {
            if let Ok(meta) = entry.metadata()
                && meta.is_file()
            {
                total += meta.len();
            }
        }
    }
    Ok(total)
}

/// Dossiers d'un dossier du daemon (services, volumes).
fn read_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}
