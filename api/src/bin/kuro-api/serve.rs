//! `kuro-api serve`

use std::sync::Arc;

use anyhow::Context;
use kuro_api::cluster::Bus;
use kuro_api::config::Config;
use kuro_api::enrollment::http::{EnrollmentSettings, EnrollmentState};
use kuro_api::iam::application::IamState;
use kuro_api::nodes::application::{Instance, Presence};
use kuro_api::nodes::infra::NodeRepository;
use kuro_api::pki::infra::PkiFiles;
use kuro_api::prefabs::application as prefabs;
use kuro_api::servers::application::{ConsoleAccess, ConsoleHub, LinkEvents, ServersState};
use kuro_api::state::AppState;
use kuro_api::ws::{CommandRelay, DaemonListener, LinkRegistry};
use kuro_api::{db, http};
use tokio_util::sync::CancellationToken;

pub async fn run(config: Config) -> anyhow::Result<()> {
    kuro_runtime::telemetry::init(&config.log)?;
    if !config.auth.require_totp {
        tracing::warn!(
            "auth.require_totp is disabled: admin sessions open without TOTP (dev only)"
        );
    }

    let ca = PkiFiles::from(&config.daemon)
        .load_ca()
        .context("loading CA")?;
    let pool = db::connect(&config.database)
        .await
        .context("connecting to database")?;
    db::migrate(&pool).await.context("running migrations")?;

    let imported = prefabs::import_dir(&pool, &config.prefabs.dir)
        .await
        .context("importing prefabs")?;
    for (folder, error) in &imported.invalid {
        tracing::error!(folder, %error, "invalid prefab skipped");
    }
    for prefab in &imported.conflicts {
        // Contenu modifié sans changement de version : la version stockée reste la référence.
        tracing::warn!(
            prefab,
            "prefab changed without a version bump, stored version kept"
        );
    }
    tracing::info!(
        imported = imported.imported.len(),
        unchanged = imported.unchanged.len(),
        dir = %config.prefabs.dir.display(),
        "prefabs loaded"
    );

    // Cette instance de l'API (ADR-0037) : inscrite, puis battement et balayage en tâche de fond.
    let instance = Instance::start(pool.clone())
        .await
        .context("registering this api instance")?;
    tracing::info!(instance = %instance.id, "api instance registered");
    let presence = Presence::new(NodeRepository::new(pool.clone()), instance.clone());
    let heartbeats = presence.clone();
    let stale = presence.reset().await.context("resetting node presence")?;
    tracing::info!(nodes = stale, "stale online nodes marked offline");

    let settings = EnrollmentSettings {
        token_ttl: config.enrollment.token_ttl(),
        ca_fingerprint: ca.fingerprint(),
        daemon_port: config.daemon.bind.port(),
    };
    let enrollment = EnrollmentState {
        pool: pool.clone(),
        ca: Arc::new(ca),
        certificate_validity: config.enrollment.certificate_validity(),
    };
    // Registre des sessions : partagé entre le listener et le REST (révocation, suppression).
    let links = Arc::new(LinkRegistry::default());
    // Commandes : au registre local, ou relayées à l'instance qui tient le nœud (ADR-0037).
    let relay = CommandRelay::new(links.clone(), pool.clone(), instance.id);
    // Spectateurs des consoles : partagés entre le lien (console reçue) et le REST (WebSocket).
    let console = ConsoleHub::new(relay.clone());
    let server_events = Arc::new(LinkEvents::new(
        pool.clone(),
        console.clone(),
        relay.clone(),
    ));
    let daemon_listener = DaemonListener::bind(
        &config.daemon,
        presence,
        enrollment,
        links.clone(),
        server_events.clone(),
    )
    .await
    .context("starting daemon listener")?;
    let iam = IamState::new(pool.clone(), &config.auth)?;
    let servers = ServersState {
        pool: pool.clone(),
        commands: relay.clone(),
        secrets_key: *iam.keys.server_secrets(),
        console,
        console_access: ConsoleAccess {
            max_session: std::time::Duration::from_secs(config.auth.access_ttl_secs),
        },
        content: kuro_api::content::ContentSources {
            modrinth: Arc::new(
                kuro_api::content::modrinth::Modrinth::new(&config.content.modrinth_url)
                    .map_err(|e| anyhow::anyhow!("content provider: {e}"))?,
            ),
            curseforge: Arc::new(
                kuro_api::content::curseforge::CurseForge::new(&config.content.curseforge_url)
                    .map_err(|e| anyhow::anyhow!("content provider: {e}"))?,
            ),
            steam: Arc::new(
                kuro_api::content::steam::SteamWorkshop::new(&config.content.steam_url)
                    .map_err(|e| anyhow::anyhow!("content provider: {e}"))?,
            ),
            steam_store: Arc::new(
                kuro_api::content::steam_store::SteamStoreClient::new(&config.content.steam_url)
                    .map_err(|e| anyhow::anyhow!("steam store: {e}"))?,
            ),
            steam_grid: Arc::new(
                kuro_api::content::steamgriddb::SteamGridDb::new(&config.content.steamgriddb_url)
                    .map_err(|e| anyhow::anyhow!("steamgriddb: {e}"))?,
            ),
        },
    };
    // Le lien redéploie après des paquets extraits (ADR-0030) : il reçoit les cas d'usage.
    server_events.attach(servers.clone());
    // Clés d'API des sources (ADR-0029), réglées dans le panel.
    let content_sources = kuro_api::content::settings::ContentSourcesState {
        pool: pool.clone(),
        key: *iam.keys.server_secrets(),
        sources: servers.content.clone(),
    };
    kuro_api::content::settings::load(&content_sources)
        .await
        .context("loading content source keys")?;
    let panel = config.http.panel_dir.as_deref().map(|dir| {
        tracing::info!(dir = %dir.display(), "serving the panel");
        kuro_api::http::panel::Panel::new(dir)
    });
    // Serveur MCP (ADR-0023) : configuré par son URL publique, activé dans les paramètres.
    let metadata = Arc::new(kuro_api::oauth::metadata::HttpsFetcher::new().context("tls client")?);
    let oauth = config
        .mcp
        .public_url
        .as_deref()
        .map(|url| {
            kuro_api::oauth::application::OAuthState::new(
                pool.clone(),
                url,
                config.mcp.panel_url.as_deref(),
                metadata.clone(),
            )
        })
        .transpose()
        .map_err(|e| anyhow::anyhow!("mcp: {e}"))?;
    let dns = kuro_api::dns::application::DnsState::new(
        pool.clone(),
        *iam.keys.server_secrets(),
        Arc::new(kuro_api::dns::cloudflare::Cloudflare::new().context("tls client")?),
    );
    let state = AppState {
        content_sources,
        dns: dns.clone(),
        oauth,
        panel,
        iam,
        enrollment: settings,
        links: relay.clone(),
        servers,
        pool,
        prefabs_dir: prefabs::PrefabsDir(config.prefabs.dir.clone()),
        http: kuro_api::http::policy::HttpPolicy::from_config(&config.http),
        readiness: daemon_listener.readiness(),
    };

    let shutdown = CancellationToken::new();
    tokio::spawn({
        let shutdown = shutdown.clone();
        async move {
            kuro_runtime::shutdown::signal().await;
            shutdown.cancel();
        }
    });

    // Développement : le dossier des prefabs officiels est réimporté dès qu'il change.
    if config.prefabs.watch {
        tokio::spawn(prefabs::watch_dir(
            state.pool.clone(),
            config.prefabs.dir.clone(),
            shutdown.clone(),
        ));
    }

    // Battement de cette instance et nettoyage des instances mortes (ADR-0037).
    tokio::spawn(instance.clone().run_forever(shutdown.clone()));
    // Heartbeats des nœuds écrits par lots.
    tokio::spawn(heartbeats.flush_forever(shutdown.clone()));

    // Bus entre instances (ADR-0037) : commandes relayées, clés des sources rechargées.
    let bus = relay.subscribe(Bus::new(state.pool.clone())).subscribe(
        kuro_api::content::settings::CHANGES_CHANNEL,
        Arc::new({
            let sources = state.content_sources.clone();
            move |_payload: &str| {
                let sources = sources.clone();
                tokio::spawn(async move {
                    if let Err(error) = kuro_api::content::settings::load(&sources).await {
                        tracing::warn!(%error, "failed to reload content source keys");
                    }
                });
            }
        }),
    );
    tokio::spawn(bus.run_forever(shutdown.clone()));

    // Tâches planifiées des serveurs (ADR-0022).
    tokio::spawn(kuro_api::servers::application::scheduler::run_forever(
        state.servers.clone(),
        shutdown.clone(),
    ));

    // Essais du Studio : suppression à l'expiration (ADR-0011).
    tokio::spawn(kuro_api::studio::trials::sweep_forever(
        state.servers.clone(),
        shutdown.clone(),
    ));

    // DNS des serveurs et des nœuds : réconciliation (ADR-0026).
    tokio::spawn(kuro_api::dns::application::run_forever(
        dns,
        shutdown.clone(),
    ));

    // Conservation du journal d'audit (ADR-0019).
    tokio::spawn(kuro_api::audit::application::purge_daily(
        state.pool.clone(),
        shutdown.clone(),
    ));

    let (http, ()) = tokio::join!(
        http::server::serve(&config.http, state, shutdown.clone()),
        daemon_listener.run(shutdown.clone()),
    );
    http.context("running http server")
}
