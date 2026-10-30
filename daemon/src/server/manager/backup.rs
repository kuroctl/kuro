//! Sauvegardes des serveurs (ADR-0018).
//!
//! Une sauvegarde ne bloque pas le serveur (on peut l'arrêter ou le démarrer pendant
//! l'archivage), mais elle exclut déploiement, installation, restauration et suppression. Sa fin
//! part dans `backup.done`.

use std::time::Duration;

use base64::Engine;
use futures_util::StreamExt;
use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::api::{BackupCreate, BackupHook, RestoreMode};
use kuro_protocol::daemon::server::ServerState;
use kuro_protocol::daemon::{
    BackupChunk, BackupDone, BackupInfo, BackupListing, BackupOutcome, BackupState,
};
use kuro_protocol::limits::is_valid_command;
use regex::Regex;
use uuid::Uuid;

use super::disk::ensure_active;
use super::{CommandError, Manager, owner_of, unix_now};
use crate::backup::pattern::Ignore;
use crate::backup::{BackupError, Store, archive};
use crate::docker::container_name;
use crate::fs::volumes::Owner;
use crate::install::{InstallError, extract};

/// Attente maximale de la ligne `wait` d'une commande.
const HOOK_TIMEOUT: Duration = Duration::from_secs(60);

impl Manager {
    fn backups(&self, server_id: Uuid) -> Store {
        Store::new(self.dir(server_id).root())
    }

    /// Lance l'archivage ; acceptée tout de suite, terminée par `backup.done`.
    pub async fn backup_create(&self, create: BackupCreate) -> Result<(), CommandError> {
        let deploy = self.spec(create.server_id)?;
        ensure_active(&deploy)?;
        self.check_quota(&deploy, 0).await?;
        let ignore = Ignore::new(&create.ignore)
            .ok_or_else(|| CommandError::InvalidSpec("invalid ignore pattern".to_owned()))?;
        for hook in create.before.iter().chain(&create.after) {
            if !is_valid_command(&hook.console)
                || hook.wait.as_deref().is_some_and(|w| Regex::new(w).is_err())
            {
                return Err(CommandError::InvalidSpec("invalid backup hook".to_owned()));
            }
        }
        let guard = self.acquire_archiving(create.server_id, create.backup_id)?;
        let this = self.clone();
        tokio::spawn(async move {
            // Sauvegardes limitées en nombre sur l'hôte : les suivantes attendent leur tour.
            let _slot = this.inner.backup_slots.clone().acquire_owned().await;
            let outcome = this.run_backup(&deploy, &create, ignore).await;
            drop(guard);
            let outcome = match outcome {
                Ok(meta) => BackupOutcome::Ok {
                    size: meta.size,
                    sha256: meta.sha256,
                },
                Err(error) => {
                    tracing::warn!(server_id = %create.server_id, %error, "backup failed");
                    this.install_log(create.server_id, format!("backup failed: {error}"));
                    BackupOutcome::Failed {
                        reason: error.to_string(),
                    }
                }
            };
            let done = BackupDone {
                server_id: create.server_id,
                backup_id: create.backup_id,
                outcome,
            };
            // Lien coupé : l'API relira l'état avec `backup.list` à la reconnexion.
            let _ = this.inner.backups.send(done).await;
            this.measure_later(create.server_id);
        });
        Ok(())
    }

    async fn run_backup(
        &self,
        deploy: &ServerDeploy,
        create: &BackupCreate,
        ignore: Ignore,
    ) -> Result<crate::backup::Meta, BackupError> {
        let id = create.server_id;
        self.install_log(id, "backup started".to_owned());
        let hooked = self.run_hooks(deploy, &create.before).await;
        let store = self.backups(id);
        let volumes = self.volumes(id);
        let roots: Vec<[String; 2]> = deploy
            .services
            .iter()
            .flat_map(|s| s.volumes.iter().map(|v| [s.name.clone(), v.name.clone()]))
            .collect();
        let backup_id = create.backup_id;
        let outcome = tokio::task::spawn_blocking(move || {
            let out = store.begin(backup_id)?;
            match archive::create(&volumes, &roots, &ignore, out) {
                Ok((meta, archived)) => {
                    store.finish(backup_id, &meta)?;
                    Ok((meta, archived))
                }
                Err(error) => {
                    store.abort(backup_id);
                    Err(error)
                }
            }
        })
        .await
        .map_err(|e| BackupError::Io(std::io::Error::other(e.to_string())))?;
        // Après l'archivage, même en échec : le jeu reprend ses écritures.
        if hooked {
            self.run_hooks(deploy, &create.after).await;
        }
        let (meta, archived) = outcome?;
        self.install_log(
            id,
            format!(
                "backup finished: {} files, {} bytes archived into {} bytes",
                archived.files, archived.bytes, meta.size
            ),
        );
        Ok(meta)
    }

    /// Commandes console, serveur en marche seulement ; retourne si elles ont été envoyées.
    async fn run_hooks(&self, deploy: &ServerDeploy, hooks: &[BackupHook]) -> bool {
        if hooks.is_empty() {
            return true;
        }
        let id = deploy.server_id;
        let main = container_name(id, &deploy.lifecycle.service);
        let docker = &self.inner.docker;
        if !docker
            .status(&main)
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s.running)
        {
            return false;
        }
        for hook in hooks {
            let lines = hook
                .wait
                .as_deref()
                .and_then(|w| Regex::new(w).ok())
                .map(|pattern| (pattern, Box::pin(docker.follow_lines(&main, unix_now()))));
            if let Err(error) = docker.send_line(&main, &hook.console).await {
                tracing::warn!(server_id = %id, %error, "backup hook failed");
                continue;
            }
            let Some((pattern, mut lines)) = lines else {
                continue;
            };
            let waited = tokio::time::timeout(HOOK_TIMEOUT, async {
                while let Some(line) = lines.next().await {
                    if pattern.is_match(&line) {
                        return;
                    }
                }
            })
            .await;
            if waited.is_err() {
                tracing::warn!(server_id = %id, command = %hook.console, "backup hook wait timed out");
            }
        }
        true
    }

    /// Restaure une sauvegarde ; le serveur est arrêté d'abord. Acceptée tout de suite ; l'état
    /// suit (`restoring`, puis `offline` ou `failed`).
    pub async fn backup_restore(
        &self,
        server_id: Uuid,
        backup_id: Uuid,
        mode: RestoreMode,
    ) -> Result<(), CommandError> {
        let deploy = self.spec(server_id)?;
        ensure_active(&deploy)?;
        self.check_quota(&deploy, 0).await?;
        match self.backups(server_id).get(backup_id) {
            Ok(_) => {}
            Err(BackupError::NotFound) => return Err(CommandError::NotFound),
            Err(error) => return Err(CommandError::Internal(error.to_string())),
        }
        let guard = self.acquire(server_id)?;
        let this = self.clone();
        tokio::spawn(async move {
            let _slot = this.inner.backup_slots.clone().acquire_owned().await;
            match this.run_restore(&deploy, backup_id, mode).await {
                Ok(()) => {
                    this.install_log(server_id, "restore finished".to_owned());
                    this.report(server_id, ServerState::Offline, None);
                }
                Err(error) => {
                    tracing::warn!(%server_id, %error, "restore failed");
                    this.install_log(server_id, format!("restore failed: {error}"));
                    this.report(
                        server_id,
                        ServerState::Failed,
                        Some(format!("restore: {error}")),
                    );
                }
            }
            drop(guard);
            this.measure_later(server_id);
        });
        Ok(())
    }

    async fn run_restore(
        &self,
        deploy: &ServerDeploy,
        backup_id: Uuid,
        mode: RestoreMode,
    ) -> Result<(), InstallError> {
        let id = deploy.server_id;
        let main = container_name(id, &deploy.lifecycle.service);
        let running = self
            .inner
            .docker
            .status(&main)
            .await
            .map_err(failed)?
            .is_some_and(|s| s.running);
        if running {
            self.stop_all(deploy).await.map_err(failed)?;
        }
        self.report(id, ServerState::Restoring, None);
        let volumes = self.volumes(id);
        let store = self.backups(id);
        // Propriétaire de chaque volume encore déclaré ; les autres entrées sont ignorées.
        let mut owners = Vec::new();
        for service in &deploy.services {
            let (uid, gid) = owner_of(deploy, &service.name).map_err(failed)?;
            for volume in &service.volumes {
                owners.push((
                    service.name.clone(),
                    volume.name.clone(),
                    Owner { uid, gid },
                ));
            }
        }
        self.install_log(id, format!("restoring backup {backup_id}"));
        let extracted = tokio::task::spawn_blocking(move || {
            if mode == RestoreMode::Clean {
                for (service, volume, _) in &owners {
                    volumes.clear([service.as_str(), volume.as_str()])?;
                }
            }
            let archive = store.open(backup_id).map_err(failed)?;
            let owner = |path: &[&str]| match path {
                [service, volume, _, ..] => owners
                    .iter()
                    .find(|(s, v, _)| s == service && v == volume)
                    .map(|(_, _, owner)| *owner),
                _ => None,
            };
            extract::restore(&volumes, archive, &owner)
        })
        .await
        .map_err(failed)??;
        // Comme au déploiement : tout le contenu restauré revient à l'utilisateur du service
        // (dossiers recréés compris, quel que soit l'hôte).
        self.give_volumes(deploy).await.map_err(failed)?;
        self.install_log(
            id,
            format!(
                "{} files, {} bytes restored ({} entries skipped)",
                extracted.files, extracted.bytes, extracted.skipped
            ),
        );
        Ok(())
    }

    /// Supprime une sauvegarde ; refusée pendant son archivage.
    pub async fn backup_delete(
        &self,
        server_id: Uuid,
        backup_id: Uuid,
    ) -> Result<(), CommandError> {
        if self.archiving(server_id) == Some(backup_id) {
            return Err(CommandError::Busy);
        }
        let store = self.backups(server_id);
        blocking(move || store.delete(backup_id)).await?;
        self.measure_later(server_id);
        Ok(())
    }

    /// Sauvegardes terminées, et celle en cours. Nettoie au passage les archives interrompues.
    pub async fn backup_list(&self, server_id: Uuid) -> Result<BackupListing, CommandError> {
        let current = self.archiving(server_id);
        let store = self.backups(server_id);
        let found = blocking(move || {
            store.clean(current)?;
            store.list()
        })
        .await?;
        let mut backups: Vec<BackupInfo> = found
            .into_iter()
            .map(|(backup_id, meta)| BackupInfo {
                backup_id,
                state: BackupState::Ready {
                    size: meta.size,
                    sha256: meta.sha256,
                },
            })
            .collect();
        if let Some(backup_id) = current {
            backups.push(BackupInfo {
                backup_id,
                state: BackupState::Creating,
            });
        }
        Ok(BackupListing { backups })
    }

    /// Morceau d'une archive terminée, en base64.
    pub async fn backup_read(
        &self,
        server_id: Uuid,
        backup_id: Uuid,
        offset: u64,
    ) -> Result<BackupChunk, CommandError> {
        ensure_active(&self.spec(server_id)?)?;
        let store = self.backups(server_id);
        let (data, eof) = blocking(move || store.read(backup_id, offset)).await?;
        Ok(BackupChunk {
            data: base64::engine::general_purpose::STANDARD.encode(data),
            eof,
        })
    }

    /// Sauvegarde en cours pour ce serveur.
    fn archiving(&self, server_id: Uuid) -> Option<Uuid> {
        self.inner.archiving().get(&server_id).copied()
    }

    /// Réserve l'archivage : refusé pendant une autre opération ou une autre sauvegarde.
    fn acquire_archiving(
        &self,
        server_id: Uuid,
        backup_id: Uuid,
    ) -> Result<ArchivingGuard, CommandError> {
        // Le verrou `busy` est pris en premier, comme dans `acquire`, pour un ordre unique.
        let busy = self.inner.busy();
        let mut archiving = self.inner.archiving();
        if busy.contains(&server_id) || archiving.contains_key(&server_id) {
            return Err(CommandError::Busy);
        }
        archiving.insert(server_id, backup_id);
        Ok(ArchivingGuard {
            manager: self.clone(),
            server_id,
        })
    }
}

/// Libère l'archivage du serveur.
struct ArchivingGuard {
    manager: Manager,
    server_id: Uuid,
}

impl Drop for ArchivingGuard {
    fn drop(&mut self) {
        self.manager.inner.archiving().remove(&self.server_id);
    }
}

impl From<BackupError> for CommandError {
    fn from(error: BackupError) -> Self {
        match error {
            BackupError::NotFound => Self::NotFound,
            other => Self::Internal(other.to_string()),
        }
    }
}

fn failed(error: impl std::fmt::Display) -> InstallError {
    InstallError::Failed(error.to_string())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, BackupError> + Send + 'static,
) -> Result<T, CommandError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| CommandError::Internal(e.to_string()))?
        .map_err(CommandError::from)
}
