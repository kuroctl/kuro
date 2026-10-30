//! Installation et réinstallation des serveurs (ADR-0017).
//!
//! Les étapes `download`, `extract`, `copy` et `remove` sont exécutées par le daemon, à travers
//! l'accès sûr aux volumes ; `steamcmd` et `script` dans un conteneur d'installation éphémère. La
//! sortie est diffusée sur la console quand quelqu'un la regarde.

use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use kuro_protocol::DaemonMessage;
use kuro_protocol::api::install::{
    InstallMode, InstallSpec, InstallStep, SteamLogin, SteamPlatform,
};
use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::daemon::server::ServerState;
use kuro_protocol::daemon::{ConsoleLine, ConsoleOutput};
use kuro_protocol::limits::{path_segments, truncate_console_line};
use uuid::Uuid;

use super::disk::ensure_active;
use super::{CommandError, Manager, owner_of};
use crate::docker::{ConsoleFrom, ContainerFile, Installer, container_name};
use crate::fs::volumes::{Owner, VolumeError, Volumes};
use crate::install::extract::Budget;
use crate::install::{InstallError, download, extract};

/// SteamCMD : utilisateur `steam` (uid 1000, celui des services par défaut), épinglé par empreinte.
pub(crate) const STEAMCMD_IMAGE: &str = "docker.io/cm2network/steamcmd:steam-bookworm@sha256:6420fb40313fc00ee0d7fe1ae02c4c32f2b085980f5795b7171022d3409e29fb";
const STEAMCMD: &str = "/home/steam/steamcmd/steamcmd.sh";
/// Script de connexion de SteamCMD (ADR-0033), déposé dans le conteneur d'installation : le mot
/// de passe n'apparaît ni dans les arguments, ni dans l'environnement, ni sur l'hôte.
const STEAM_LOGIN_DIR: &str = "/tmp";
const STEAM_LOGIN_FILE: &str = "kuro-steam-login.txt";
/// Durée maximale d'une étape en conteneur.
const CONTAINER_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const INSTALLER_PIDS: u32 = 1024;
/// Essais d'une étape SteamCMD sortie en erreur.
const STEAMCMD_ATTEMPTS: u32 = 3;

impl Manager {
    /// Réinstalle (étapes sur les données existantes) ou réinitialise (volumes vidés, puis
    /// étapes). Le serveur est arrêté d'abord. Acceptée tout de suite ; l'état suit.
    pub async fn install(
        &self,
        server_id: Uuid,
        mode: InstallMode,
        install: Option<InstallSpec>,
    ) -> Result<(), CommandError> {
        let deploy = self.spec(server_id)?;
        ensure_active(&deploy)?;
        self.check_quota(&deploy, 0).await?;
        if let Some(spec) = &install {
            check(&deploy, spec).map_err(|e| CommandError::InvalidSpec(e.to_string()))?;
        }
        let guard = self.acquire(server_id)?;
        let this = self.clone();
        tokio::spawn(async move {
            // Installations limitées en nombre sur l'hôte : les suivantes attendent leur tour.
            let _slot = this.inner.installs.clone().acquire_owned().await;
            let outcome = this.run_reinstall(&deploy, mode, install.as_ref()).await;
            this.finish_install(server_id, outcome);
            drop(guard);
        });
        Ok(())
    }

    async fn run_reinstall(
        &self,
        deploy: &ServerDeploy,
        mode: InstallMode,
        install: Option<&InstallSpec>,
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
        self.report(id, ServerState::Installing, None);
        if mode == InstallMode::Reset {
            let volumes = self.volumes(id);
            for service in &deploy.services {
                for volume in &service.volumes {
                    self.install_log(id, format!("clearing {}/{}", service.name, volume.name));
                    let (volumes, service, volume) =
                        (volumes.clone(), service.name.clone(), volume.name.clone());
                    blocking(move || Ok(volumes.clear([service.as_str(), volume.as_str()])?))
                        .await?;
                }
            }
        }
        match install {
            Some(spec) => self.run_steps(deploy, spec).await,
            None => Ok(()),
        }
    }

    /// Fin d'une installation : `offline` (prêt à démarrer) ou `failed` avec la cause.
    pub(super) fn finish_install(&self, server_id: Uuid, outcome: Result<(), InstallError>) {
        self.measure_later(server_id);
        match outcome {
            Ok(()) => {
                self.install_log(server_id, "installation finished".to_owned());
                self.report(server_id, ServerState::Offline, None);
            }
            Err(error) => {
                tracing::warn!(%server_id, %error, "installation failed");
                self.install_log(server_id, format!("installation failed: {error}"));
                self.report(
                    server_id,
                    ServerState::Failed,
                    Some(format!("install: {error}")),
                );
            }
        }
    }

    /// Exécute les étapes dans l'ordre ; la première en échec arrête tout.
    pub(super) async fn run_steps(
        &self,
        deploy: &ServerDeploy,
        spec: &InstallSpec,
    ) -> Result<(), InstallError> {
        let id = deploy.server_id;
        check(deploy, spec)?;
        self.report(id, ServerState::Installing, None);
        let (uid, gid) = owner_of(deploy, &spec.service).map_err(failed)?;
        let owner = Owner { uid, gid };
        let total = spec.steps.len();
        let mut budget = self.extraction_budget(deploy);
        for (index, step) in spec.steps.iter().enumerate() {
            self.install_log(
                id,
                format!("step {}/{total}: {}", index + 1, describe(step)),
            );
            self.run_step(deploy, spec, owner, step, &mut budget)
                .await?;
        }
        Ok(())
    }

    /// Plafond d'extraction d'une opération : celui d'une installation, ramené au quota disque
    /// restant du serveur.
    pub(super) fn extraction_budget(&self, deploy: &ServerDeploy) -> Budget {
        let budget = Budget::install();
        if deploy.disk_bytes == 0 {
            return budget;
        }
        let used = self
            .inner
            .disk()
            .get(&deploy.server_id)
            .copied()
            .unwrap_or(0);
        budget.capped(deploy.disk_bytes.saturating_sub(used))
    }

    async fn run_step(
        &self,
        deploy: &ServerDeploy,
        spec: &InstallSpec,
        owner: Owner,
        step: &InstallStep,
        budget: &mut Budget,
    ) -> Result<(), InstallError> {
        let id = deploy.server_id;
        let volumes = self.volumes(id);
        let path = |container: &str| segments(deploy, &spec.service, container);
        match step {
            InstallStep::Download { url, sha256, to } => {
                let target = path(to)?;
                let size = self
                    .download(
                        volumes,
                        target,
                        owner,
                        url,
                        download::Expected::Sha256(sha256),
                    )
                    .await?;
                self.install_log(id, format!("downloaded {size} bytes, sha256 verified"));
            }
            InstallStep::Extract {
                from,
                to,
                subdir,
                executable,
            } => {
                let (from, to) = (path(from)?, path(to)?);
                let (subdir, executable) = (subdir.clone(), executable.clone());
                let mut left = *budget;
                let (extracted, left) = blocking(move || {
                    let extracted = extract::extract(
                        &volumes,
                        &borrowed(&from),
                        &borrowed(&to),
                        subdir.as_deref(),
                        &executable,
                        owner,
                        &mut left,
                    )?;
                    Ok((extracted, left))
                })
                .await?;
                *budget = left;
                self.install_log(
                    id,
                    format!(
                        "extracted {} files ({} bytes), {} links skipped",
                        extracted.files, extracted.bytes, extracted.skipped
                    ),
                );
            }
            InstallStep::Copy { content, to } => {
                let to = path(to)?;
                let content = content.clone();
                blocking(move || {
                    let to = borrowed(&to);
                    volumes.mkdir_all(&to[..to.len() - 1], owner)?;
                    Ok(volumes.write(&to, &content, None, owner)?)
                })
                .await?;
            }
            InstallStep::Remove { path: target } => {
                let target = path(target)?;
                blocking(move || match volumes.delete(&borrowed(&target)) {
                    Ok(()) | Err(VolumeError::NotFound) => Ok(()),
                    Err(error) => Err(error.into()),
                })
                .await?;
            }
            InstallStep::Steamcmd {
                app,
                dir,
                beta,
                validate,
                platform,
                login,
            } => {
                let target = path(dir)?;
                blocking(move || Ok(volumes.mkdir_all(&borrowed(&target), owner)?)).await?;
                let script = login.as_ref().map(login_script).transpose()?;
                let command = steamcmd_command(
                    *app,
                    dir,
                    beta.as_deref(),
                    *validate,
                    *platform,
                    script.is_some(),
                );
                let user = numeric_owner(&service_of(deploy, spec)?.user);
                let extras = Extras {
                    file: script.as_deref().map(|script| ContainerFile {
                        dir: STEAM_LOGIN_DIR,
                        name: STEAM_LOGIN_FILE,
                        content: script.as_bytes(),
                        owner: user.unwrap_or((0, 0)),
                        // Lisible par l'utilisateur du conteneur seulement, s'il est numérique.
                        mode: if user.is_some() { 0o400 } else { 0o444 },
                    }),
                    secret: login.as_ref().map(|l| l.password.as_str()),
                    abort: login.is_some().then_some(steam_login_failure),
                };
                // SteamCMD échoue parfois au premier passage (« Missing configuration ») et réussit
                // au suivant : relancé quand il sort en erreur, jamais après un dépassement de délai.
                for attempt in 1..=STEAMCMD_ATTEMPTS {
                    let env = vec!["HOME=/home/steam".to_owned()];
                    match self
                        .run_container(deploy, spec, STEAMCMD_IMAGE, command.clone(), env, &extras)
                        .await
                    {
                        Err(InstallError::Exited(code)) if attempt < STEAMCMD_ATTEMPTS => {
                            self.install_log(
                                deploy.server_id,
                                format!(
                                    "steamcmd exited with code {code}, retrying ({}/{STEAMCMD_ATTEMPTS})",
                                    attempt + 1
                                ),
                            );
                        }
                        outcome => {
                            outcome?;
                            break;
                        }
                    }
                }
            }
            InstallStep::Script { run, image } => {
                let image = image.as_deref().unwrap_or(&spec.image);
                let command = vec!["sh".to_owned(), "-c".to_owned(), run.clone()];
                let env = vec!["HOME=/tmp".to_owned()];
                self.run_container(deploy, spec, image, command, env, &Extras::default())
                    .await?;
            }
        }
        Ok(())
    }

    pub(super) async fn download(
        &self,
        volumes: Volumes,
        target: Vec<String>,
        owner: Owner,
        url: &str,
        expected: download::Expected<'_>,
    ) -> Result<u64, InstallError> {
        let (pending, file) = blocking(move || {
            let path = borrowed(&target);
            volumes.mkdir_all(&path[..path.len() - 1], owner)?;
            let pending = volumes.create(&path, owner, 0o644)?;
            let file = pending.file.try_clone().map_err(VolumeError::from)?;
            Ok((pending, file))
        })
        .await?;
        let mut file = tokio::fs::File::from_std(file);
        // Abandonné (erreur, empreinte fausse) : le fichier temporaire disparaît avec `pending`.
        let size = download::fetch(&self.inner.downloads, url, expected, &mut file).await?;
        drop(file);
        blocking(move || Ok(pending.commit()?)).await?;
        Ok(size)
    }

    /// Étape en conteneur : volumes du service montés, sortie diffusée, durée bornée.
    async fn run_container(
        &self,
        deploy: &ServerDeploy,
        spec: &InstallSpec,
        image: &str,
        command: Vec<String>,
        env: Vec<String>,
        extras: &Extras<'_>,
    ) -> Result<(), InstallError> {
        let id = deploy.server_id;
        let docker = &self.inner.docker;
        let service = service_of(deploy, spec)?;
        let dir = self.dir(id);
        let mounts: Vec<(String, PathBuf)> = service
            .volumes
            .iter()
            .map(|v| (v.path.clone(), dir.volume_dir(&service.name, &v.name)))
            .collect();
        docker.ensure_image(image).await.map_err(failed)?;
        docker.ensure_network(id).await.map_err(failed)?;
        let name = docker
            .create_installer(&Installer {
                server_id: id,
                image,
                command,
                env,
                user: &service.user,
                mounts,
                memory_bytes: service.limits.memory_bytes,
                cpu_millis: service.limits.cpu_millis,
                pids: INSTALLER_PIDS,
            })
            .await
            .map_err(failed)?;
        // Ligne relayée : jamais le secret de l'étape, même si un programme l'écrit.
        let masked = |text: String| match extras.secret {
            Some(secret) if text.contains(secret) => text.replace(secret, "********"),
            _ => text,
        };
        let outcome = async {
            if let Some(file) = &extras.file {
                docker.put_file(&name, file).await.map_err(failed)?;
            }
            docker.start(&name).await.map_err(failed)?;
            // `fuse` : le flux peut finir avant le conteneur, il est alors relu sans risque.
            let mut lines = Box::pin(docker.console_lines(&name, ConsoleFrom::Since(0)).fuse());
            let wait = docker.wait(&name);
            tokio::pin!(wait);
            let deadline = tokio::time::sleep(CONTAINER_TIMEOUT);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    () = &mut deadline => {
                        let _ = docker.kill(&name).await;
                        return Err(InstallError::Failed("install step timed out".to_owned()));
                    }
                    Some(line) = lines.next() => {
                        let text = masked(line.text);
                        // Échec sans issue (compte refusé, Steam Guard) : inutile d'attendre.
                        let abort = extras.abort.and_then(|abort| abort(&text));
                        self.install_log(id, text);
                        if let Some(reason) = abort {
                            let _ = docker.kill(&name).await;
                            return Err(InstallError::Failed(reason.to_owned()));
                        }
                    }
                    code = &mut wait => {
                        // Les dernières lignes, arrivées avec la fin du conteneur.
                        while let Ok(Some(line)) =
                            tokio::time::timeout(Duration::from_millis(200), lines.next()).await
                        {
                            self.install_log(id, masked(line.text));
                        }
                        return match code.map_err(failed)? {
                            0 => Ok(()),
                            code => Err(InstallError::Exited(code)),
                        };
                    }
                }
            }
        }
        .await;
        let _ = docker.remove_container(&name).await;
        outcome
    }

    pub(super) fn volumes(&self, server_id: Uuid) -> Volumes {
        Volumes::new(self.dir(server_id).root())
    }

    /// Ligne de l'installation : journal du daemon, et console si quelqu'un la regarde.
    pub(super) fn install_log(&self, server_id: Uuid, text: String) {
        // Chaque ligne du programme installé : journal technique en `debug`, pas en `info`.
        tracing::debug!(%server_id, "install: {text}");
        if !self.inner.watches().contains_key(&server_id) {
            return;
        }
        let at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let output = DaemonMessage::ConsoleLine(ConsoleOutput {
            server_id,
            lines: vec![ConsoleLine {
                at_ms,
                text: truncate_console_line(format!("[kuro] {text}")),
            }],
            dropped: 0,
        });
        let _ = self.inner.streams.try_send(output);
    }
}

/// Les étapes visent un service connu et des chemins sous ses volumes.
fn check(deploy: &ServerDeploy, spec: &InstallSpec) -> Result<(), InstallError> {
    for step in &spec.steps {
        let paths: Vec<&str> = match step {
            InstallStep::Download { to, .. } | InstallStep::Copy { to, .. } => vec![to],
            InstallStep::Extract { from, to, .. } => vec![from, to],
            InstallStep::Remove { path } => vec![path],
            InstallStep::Steamcmd { dir, .. } => vec![dir],
            InstallStep::Script { .. } => vec![],
        };
        for path in paths {
            segments(deploy, &spec.service, path)?;
        }
    }
    Ok(())
}

/// Chemin du conteneur → chemin des volumes (`[service, volume, …]`).
pub(super) fn segments(
    deploy: &ServerDeploy,
    service: &str,
    container: &str,
) -> Result<Vec<String>, InstallError> {
    let invalid = || {
        InstallError::Failed(format!(
            "`{container}` is not under a volume of `{service}`"
        ))
    };
    let spec = deploy
        .services
        .iter()
        .find(|s| s.name == service)
        .ok_or_else(invalid)?;
    for volume in &spec.volumes {
        let root = volume.path.trim_end_matches('/');
        let rest = if container == root {
            Some("")
        } else {
            container.strip_prefix(&format!("{root}/"))
        };
        if let Some(rest) = rest {
            let rest = path_segments(rest).ok_or_else(invalid)?;
            let mut path = vec![service.to_owned(), volume.name.clone()];
            path.extend(rest.iter().map(|s| (*s).to_owned()));
            return Ok(path);
        }
    }
    Err(invalid())
}

/// Ce qu'une étape en conteneur ajoute à sa commande.
#[derive(Default)]
struct Extras<'a> {
    /// Fichier déposé avant le démarrage.
    file: Option<ContainerFile<'a>>,
    /// Masqué dans chaque ligne relayée.
    secret: Option<&'a str>,
    /// Ligne qui met fin à l'étape, avec la raison donnée.
    abort: Option<fn(&str) -> Option<&'static str>>,
}

fn service_of<'a>(
    deploy: &'a ServerDeploy,
    spec: &InstallSpec,
) -> Result<&'a kuro_protocol::api::server::ServiceSpec, InstallError> {
    deploy
        .services
        .iter()
        .find(|s| s.name == spec.service)
        .ok_or_else(|| InstallError::Failed(format!("unknown service `{}`", spec.service)))
}

/// `uid:gid` numériques de l'utilisateur d'un conteneur, s'il est donné ainsi.
fn numeric_owner(user: &str) -> Option<(u64, u64)> {
    let (uid, gid) = user.split_once(':').unwrap_or((user, user));
    Some((uid.parse().ok()?, gid.parse().ok()?))
}

/// Script SteamCMD de connexion. L'API refuse déjà les caractères qui en sortiraient ; le daemon
/// le vérifie aussi, le script étant interprété par SteamCMD.
fn login_script(login: &SteamLogin) -> Result<String, InstallError> {
    let safe = |value: &str| {
        !value.is_empty()
            && !value
                .chars()
                .any(|c| c.is_control() || c == '"' || c == '\\')
    };
    if !safe(&login.username) || !safe(&login.password) {
        return Err(InstallError::Failed(
            "the Steam account of the installation is invalid".to_owned(),
        ));
    }
    Ok(format!(
        "login \"{}\" \"{}\"\n",
        login.username, login.password
    ))
}

/// Ligne de SteamCMD qui rend la connexion impossible : la raison, lisible dans le panel.
fn steam_login_failure(line: &str) -> Option<&'static str> {
    let line = line.to_ascii_lowercase();
    if line.contains("steam guard") || line.contains("two-factor") || line.contains("two factor") {
        Some(
            "steam_guard_required: the Steam account of the installation asks for a Steam Guard code; use a dedicated account without Steam Guard",
        )
    } else if line.contains("invalid password") || line.contains("login failure") {
        Some(
            "steam_login_failed: Steam refused the account of the installation (username or password)",
        )
    } else if line.contains("rate limit exceeded") {
        Some("steam_login_rate_limited: too many Steam logins for this account; retry later")
    } else {
        None
    }
}

fn describe(step: &InstallStep) -> String {
    match step {
        // L'hôte seulement : une URL signée (CDN) n'a rien à faire dans un journal.
        InstallStep::Download { url, to, .. } => format!("download from {} to {to}", host_of(url)),
        InstallStep::Extract { from, to, .. } => format!("extract {from} to {to}"),
        InstallStep::Copy { to, .. } => format!("write {to}"),
        InstallStep::Remove { path } => format!("remove {path}"),
        InstallStep::Steamcmd { app, dir, .. } => format!("steamcmd app {app} into {dir}"),
        InstallStep::Script { .. } => "script".to_owned(),
    }
}

/// Hôte d'une URL, sans chemin ni paramètres.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("//").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

fn failed(error: impl std::fmt::Display) -> InstallError {
    InstallError::Failed(error.to_string())
}

fn borrowed(segments: &[String]) -> Vec<&str> {
    segments.iter().map(String::as_str).collect()
}

/// Accès disque hors du runtime asynchrone.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, InstallError> + Send + 'static,
) -> Result<T, InstallError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| InstallError::Failed(e.to_string()))?
}

/// Arguments de SteamCMD pour une app. La plateforme forcée (ADR-0032) précède la connexion :
/// SteamCMD l'applique à toute la session. `account` : connexion par le script déposé dans le
/// conteneur (ADR-0033), sinon anonyme.
fn steamcmd_command(
    app: u32,
    dir: &str,
    beta: Option<&str>,
    validate: bool,
    platform: SteamPlatform,
    account: bool,
) -> Vec<String> {
    let mut command = vec![STEAMCMD.to_owned()];
    if platform == SteamPlatform::Windows {
        command.extend([
            "+@sSteamCmdForcePlatformType".to_owned(),
            "windows".to_owned(),
        ]);
    }
    command.extend(["+force_install_dir".to_owned(), dir.to_owned()]);
    if account {
        command.extend([
            "+runscript".to_owned(),
            format!("{STEAM_LOGIN_DIR}/{STEAM_LOGIN_FILE}"),
        ]);
    } else {
        command.extend(["+login".to_owned(), "anonymous".to_owned()]);
    }
    command.extend(["+app_update".to_owned(), app.to_string()]);
    if let Some(beta) = beta {
        command.extend(["-beta".to_owned(), beta.to_owned()]);
    }
    if validate {
        command.push("validate".to_owned());
    }
    command.push("+quit".to_owned());
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linux_app_is_installed_as_before() {
        assert_eq!(
            steamcmd_command(
                896_660,
                "/data/server",
                None,
                true,
                SteamPlatform::Linux,
                false
            ),
            [
                STEAMCMD,
                "+force_install_dir",
                "/data/server",
                "+login",
                "anonymous",
                "+app_update",
                "896660",
                "validate",
                "+quit"
            ]
        );
    }

    #[test]
    fn a_windows_app_forces_the_platform_before_logging_in() {
        let command = steamcmd_command(
            2_278_520,
            "/data/server",
            Some("public-test"),
            false,
            SteamPlatform::Windows,
            false,
        );

        assert_eq!(
            command,
            [
                STEAMCMD,
                "+@sSteamCmdForcePlatformType",
                "windows",
                "+force_install_dir",
                "/data/server",
                "+login",
                "anonymous",
                "+app_update",
                "2278520",
                "-beta",
                "public-test",
                "+quit"
            ]
        );
    }

    #[test]
    fn an_account_logs_in_through_the_script_never_through_the_arguments() {
        let login = SteamLogin {
            username: "kuro_servers".to_owned(),
            password: "Pa ss!word".to_owned(),
        };
        let command = steamcmd_command(
            233_780,
            "/home/container",
            None,
            true,
            SteamPlatform::Linux,
            true,
        );

        assert_eq!(
            command,
            [
                STEAMCMD,
                "+force_install_dir",
                "/home/container",
                "+runscript",
                "/tmp/kuro-steam-login.txt",
                "+app_update",
                "233780",
                "validate",
                "+quit"
            ]
        );
        assert!(!command.join(" ").contains("Pa ss!word"));
        assert_eq!(
            login_script(&login).unwrap(),
            "login \"kuro_servers\" \"Pa ss!word\"\n"
        );
    }

    #[test]
    fn a_login_that_would_break_out_of_the_script_is_refused() {
        for (username, password) in [
            ("kuro", "a\"b"),
            ("kuro", "a\\b"),
            ("kuro", "a\n+quit"),
            ("ku\"ro", "pass"),
            ("", "pass"),
        ] {
            let login = SteamLogin {
                username: username.to_owned(),
                password: password.to_owned(),
            };
            assert!(login_script(&login).is_err(), "{username:?} {password:?}");
        }
    }

    #[test]
    fn a_refused_login_ends_the_step_with_its_reason() {
        let reason = |line| steam_login_failure(line).map(|r| r.split(':').next().unwrap());

        assert_eq!(
            reason("This computer has not been authenticated for your account using Steam Guard."),
            Some("steam_guard_required")
        );
        assert_eq!(
            reason("FAILED (Two-factor code mismatch)"),
            Some("steam_guard_required")
        );
        assert_eq!(
            reason("Logging in user 'kuro_servers' to Steam Public...FAILED (Invalid Password)"),
            Some("steam_login_failed")
        );
        assert_eq!(
            reason("ERROR (Rate Limit Exceeded)"),
            Some("steam_login_rate_limited")
        );
        assert_eq!(
            reason("Update state (0x61) downloading, progress: 12.50"),
            None
        );
    }

    #[test]
    fn a_numeric_user_owns_the_login_script() {
        assert_eq!(numeric_owner("1000:1000"), Some((1000, 1000)));
        assert_eq!(numeric_owner("1000"), Some((1000, 1000)));
        assert_eq!(numeric_owner("steam"), None);
    }
}
