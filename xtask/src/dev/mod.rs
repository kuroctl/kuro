//! `cargo xtask dev` : Postgres, API, daemon et panel en une commande.
//!
//! Ordre : base → binaires → initialisation (PKI, admin, nœud) → API → enrôlement du nœud si
//! besoin → daemon → panel. Les sorties sont préfixées par process ; Ctrl+C arrête tout.
//!
//! Rechargement à chaud (sauf `--no-reload`) : une modification des sources Rust recompile et
//! relance l'API ou le daemon concerné ; l'API applique ses migrations au démarrage. Une API ou un
//! daemon qui s'arrête attend la correction suivante au lieu de tout arrêter.

mod bootstrap;
mod procs;
mod reload;
pub mod target;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Args;
use kuro_api::config::Config;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

use crate::workspace::{self, binary, node_tool, root};
use procs::{Color, Supervisor};

const API_CONFIG: &str = "api/config.example.toml";
const COMPOSE_FILE: &str = "docker-compose.dev.yml";
const PANEL_PORT: u16 = 5173;
const READY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Args)]
pub struct DevArgs {
    /// Ne lance pas le panel (API et daemon seuls).
    #[arg(long)]
    no_panel: bool,
    /// Pas de rechargement à chaud : une modification des sources Rust exige un redémarrage.
    #[arg(long)]
    no_reload: bool,
    /// Ne crée pas le compte admin de dev : l'API démarre « à configurer » et l'assistant de
    /// mise en route s'affiche (ADR-0045) ; le jeton est repris dans le récapitulatif.
    #[arg(long)]
    setup: bool,
    /// Workspace Cargo de l'API, quand ce n'est pas celui de Kuro : un dépôt qui compile Kuro
    /// avec des extensions (ADR-0051), Kuro étant son sous-module.
    #[arg(long, value_name = "DIR")]
    api_workspace: Option<PathBuf>,
    /// Binaire de l'API, et paquet du même nom (`kuro-cloud`…).
    #[arg(long, value_name = "NAME", default_value = "kuro-api")]
    api_bin: String,
    /// Base de dev à utiliser (créée si besoin) à la place de celle de la configuration : un
    /// autre environnement dans le même Postgres.
    #[arg(long, value_name = "NAME")]
    database: Option<String>,
    /// Sources en plus dont une modification recompile et relance l'API (répétable).
    #[arg(long = "watch", value_name = "PATH")]
    watch: Vec<PathBuf>,
    /// Extension du panel admin, `<nom>=<dossier>` (répétable) : `KURO_PANEL_EXTENSIONS`.
    #[arg(long = "panel-extension", value_name = "NAME=DIR")]
    panel_extensions: Vec<String>,
}

fn api_command(config_path: &std::path::Path) -> Command {
    let mut api = target::get().command();
    api.arg("--config")
        .arg(config_path)
        // Les prefabs officiels sont réimportés dès que le dossier `prefabs/` change.
        .env("KURO_API_PREFABS__WATCH", "true")
        // « Publier » dans le Studio écrit le prefab dans le sous-module `prefabs/`, à commiter
        // dans `kuroctl/prefabs` (ADR-0042).
        .env("KURO_API_PREFABS__PUBLISH_TO_DIR", "true")
        // Serveur MCP (ADR-0023) derrière le proxy de Vite : panel et API sur la même origine.
        .env(
            "KURO_API_MCP__PUBLIC_URL",
            format!("http://localhost:{PANEL_PORT}"),
        );
    api
}

fn daemon_command() -> Command {
    let mut daemon = Command::new(binary("kuro-daemon"));
    daemon
        .arg("--config")
        .arg(root().join(bootstrap::NODE_CONFIG))
        // Même format lisible que l'API ; la config écrite à l'enrôlement reste celle de prod.
        .env("KURO_DAEMON_LOG__FORMAT", "pretty")
        // Données des serveurs dans le dépôt (ignoré par git), pas dans /var/lib/kuro.
        .env(
            "KURO_DAEMON_SERVERS__DATA_ROOT",
            root().join(".kuro-dev").join("servers"),
        );
    // Hors Linux, le daemon refuse de démarrer sans confinement des fichiers (ADR-0015) : un
    // nœud de développement l'accepte, en connaissance de cause.
    #[cfg(not(target_os = "linux"))]
    daemon.env("KURO_DAEMON_SERVERS__ALLOW_INSECURE_FS", "true");
    daemon
}

pub async fn run(args: DevArgs) -> anyhow::Result<()> {
    let config_path = root().join(API_CONFIG);
    let mut config = Config::load(Some(&config_path)).context("loading api/config.example.toml")?;
    let api = api_target(&args, &config)?;
    if let Some(url) = &api.database_url {
        config.database.url = url.clone();
    }
    target::set(api);
    let panel_addr = SocketAddr::from(([127, 0, 0, 1], PANEL_PORT));
    let mut ports = vec![config.http.bind, config.daemon.bind];
    if !args.no_panel {
        ports.push(panel_addr);
    }
    ensure_free(&ports).await?;
    ensure_prefabs()?;

    step("starting Postgres");
    let mut compose = Command::new("docker");
    compose.args(["compose", "-f", COMPOSE_FILE, "up", "-d", "--wait"]);
    workspace::run(compose, "docker compose").await?;
    ensure_env_file()?;
    if let Some(name) = &args.database {
        ensure_database(&config_path, name).await?;
    }

    step(&format!("building {} and kuro-daemon", target::get().bin));
    if !reload::build().await {
        bail!("cargo build failed");
    }

    let prepared = bootstrap::prepare(&config, args.setup).await?;
    if !args.no_panel && !root().join("panel/node_modules").exists() {
        step("installing panel dependencies");
        let mut install = node_tool("corepack");
        install.args(["pnpm", "--dir", "panel", "install"]);
        workspace::run(install, "pnpm install").await?;
    }

    let mut supervisor = Supervisor::new();
    supervisor.spawn("api", Color::Cyan, api_command(&config_path))?;
    // Le listener daemon est lié avant le serveur HTTP : sonder le port HTTP suffit (une sonde
    // TCP sur le listener daemon y laisserait un faux échec de handshake TLS).
    wait_ready(&mut supervisor, "api", &[config.http.bind]).await?;

    if let Some(enrollment) = prepared.enrollment {
        step("enrolling the dev node");
        bootstrap::enroll(enrollment).await?;
    }
    supervisor.spawn("daemon", Color::Magenta, daemon_command())?;

    if !args.no_panel {
        // Vite lancé directement par node : un seul process, arrêté proprement avec les autres.
        let mut panel = Command::new("node");
        // Pages des extensions compilées avec le panel (ADR-0051 § 8).
        if let Some(extensions) = target::get().panel_env() {
            panel.env("KURO_PANEL_EXTENSIONS", extensions);
        }
        panel
            .args(["node_modules/vite/bin/vite.js", "--strictPort", "--port"])
            .arg(PANEL_PORT.to_string())
            // IPv4 explicite : `localhost` se résout en `::1` sous Windows.
            .args(["--host", "127.0.0.1"])
            .current_dir(root().join("panel"));
        supervisor.spawn("panel", Color::Green, panel)?;
        wait_ready(&mut supervisor, "panel", &[panel_addr]).await?;
    }

    println!();
    step("ready, Ctrl+C stops everything");
    if !args.no_panel {
        println!("  panel   http://localhost:{PANEL_PORT}");
    }
    println!(
        "  api     http://{} (OpenAPI: /api/openapi.json)",
        config.http.bind
    );
    if !target::get().in_kuro() {
        println!(
            "  binary  {} (Kuro with extensions), database {}",
            target::get().bin,
            args.database.as_deref().unwrap_or("from the configuration")
        );
    }
    if let Some(extensions) = target::get().panel_env() {
        println!("  panel extensions  {extensions}");
    }
    println!("  admin   {}", prepared.admin_hint);
    println!(
        "  mail    Mailpit http://localhost:8025 (SMTP 127.0.0.1:1025, chiffrement « aucun »)"
    );
    // Jeton de mise en route (ADR-0045), écrit par l'API au démarrage s'il n'y a aucun compte.
    if let Ok(token) = std::fs::read_to_string(root().join(&config.setup.token_file)) {
        println!(
            "  setup   http://localhost:{PANEL_PORT}/setup with token {}",
            token.trim()
        );
    }
    if !args.no_reload {
        println!("  reload  Rust sources watched: api and daemon rebuilt and restarted on change");
    }
    println!();

    let result = supervise(
        &mut supervisor,
        &config_path,
        config.http.bind,
        !args.no_reload,
    )
    .await;
    step("stopping");
    supervisor.shutdown().await;
    result
}

/// Jusqu'à Ctrl+C. Avec le rechargement, une modification recompile et relance ; une API ou un
/// daemon arrêté attend la correction suivante. Sans, l'arrêt d'un process arrête tout.
async fn supervise(
    supervisor: &mut Supervisor,
    config_path: &std::path::Path,
    api_addr: SocketAddr,
    reload: bool,
) -> anyhow::Result<()> {
    let mut watch = reload.then(reload::SourceWatch::new);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            exit = supervisor.next_exit() => {
                if reload && (exit.name == "api" || exit.name == "daemon") {
                    step(&format!("{exit}: fix the code, it restarts on the next change"));
                    continue;
                }
                bail!("{exit}");
            }
            changes = async {
                match watch.as_mut() {
                    Some(watch) => watch.changed().await,
                    None => std::future::pending().await,
                }
            } => {
                step("sources changed, rebuilding");
                if !reload::rebuild().await {
                    step("build failed: the running version stays, fix and save again");
                    continue;
                }
                if changes.api {
                    step("restarting api (migrations applied at startup)");
                    supervisor.stop("api").await;
                    supervisor.spawn("api", Color::Cyan, api_command(config_path))?;
                    if let Err(error) = wait_ready(supervisor, "api", &[api_addr]).await {
                        step(&format!("{error}: fix the code, it restarts on the next change"));
                        continue;
                    }
                }
                if changes.daemon {
                    step("restarting daemon");
                    supervisor.stop("daemon").await;
                    supervisor.spawn("daemon", Color::Magenta, daemon_command())?;
                }
                step("ready");
            }
        }
    }
}

/// API de ce lancement, d'après les options : `kuro-api` de Kuro par défaut.
fn api_target(args: &DevArgs, config: &Config) -> anyhow::Result<target::ApiTarget> {
    let mut api = target::ApiTarget::kuro();
    if let Some(dir) = &args.api_workspace {
        api.workspace = target::absolute(dir)?;
    }
    api.bin = args.api_bin.clone();
    api.database_url = args
        .database
        .as_deref()
        .map(|name| target::with_database(&config.database.url, name))
        .transpose()?;
    api.watch = args
        .watch
        .iter()
        .map(|path| target::absolute(path))
        .collect::<anyhow::Result<_>>()?;
    api.panel_extensions = args
        .panel_extensions
        .iter()
        .map(|entry| target::panel_extension(entry))
        .collect::<anyhow::Result<_>>()?;
    Ok(api)
}

/// Base `name` créée dans le Postgres de dev si elle n'existe pas (la base de la configuration
/// sert de point d'entrée). `name` est déjà vérifiée (`target::with_database`).
async fn ensure_database(config_path: &std::path::Path, name: &str) -> anyhow::Result<()> {
    let config = Config::load(Some(config_path))?;
    let pool = kuro_api::db::connect(&config.database)
        .await
        .context("connecting to the dev Postgres")?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(name)
            .fetch_one(&pool)
            .await?;
    if !exists {
        step(&format!("creating the database {name}"));
        // Identifiant vérifié (minuscules, chiffres, `_`) : pas de paramètre possible ici.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&pool)
            .await?;
    }
    pool.close().await;
    Ok(())
}

/// Message de l'outil, distinct des sorties des process.
pub(crate) fn step(message: &str) {
    println!("\x1b[1m[xtask ]\x1b[0m {message}");
}

/// Un port déjà pris signale presque toujours un environnement déjà lancé.
async fn ensure_free(addresses: &[SocketAddr]) -> anyhow::Result<()> {
    for address in addresses {
        if TcpListener::bind(address).await.is_err() {
            bail!("{address} is already in use: is a dev environment (or another server) running?");
        }
    }
    Ok(())
}

/// `.env` donne `DATABASE_URL` à sqlx-cli (`migrate run`, `prepare`) ; la compilation lit `.sqlx/`.
/// Le sous-module `prefabs/` (ADR-0041) doit être récupéré : sans lui, l'API démarre avec un
/// catalogue vide et les tests échouent.
fn ensure_prefabs() -> anyhow::Result<()> {
    let dir = root().join("prefabs");
    let has_prefab = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|entry| entry.path().join("prefab.yaml").is_file())
        })
        .unwrap_or(false);
    if !has_prefab {
        bail!(
            "{} holds no prefab: run `git submodule update --init` (kuroctl/prefabs)",
            dir.display()
        );
    }
    Ok(())
}

fn ensure_env_file() -> anyhow::Result<()> {
    let env = root().join(".env");
    if !env.exists() {
        std::fs::copy(root().join(".env.example"), &env).context("creating .env")?;
    }
    Ok(())
}

/// Attend que les ports répondent, ou échoue si le process s'arrête entre-temps.
async fn wait_ready(
    supervisor: &mut Supervisor,
    name: &str,
    addresses: &[SocketAddr],
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    for address in addresses {
        while TcpStream::connect(address).await.is_err() {
            if let Some(exit) = supervisor.exited() {
                bail!("{exit} before {name} was ready");
            }
            if tokio::time::Instant::now() > deadline {
                bail!("{name} not listening on {address} after {READY_TIMEOUT:?}");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    Ok(())
}
