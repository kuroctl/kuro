//! Résolution d'un prefab en `server.deploy` : ce que le daemon recevra.

use std::collections::BTreeMap;

use kuro_prefab::settings::apply;
use kuro_prefab::{
    Deployment, ResolveError, SteamAccountRequired, resolve, resolve_install, with_steam_login,
};
use kuro_protocol::api::install::{InstallStep, SteamLogin, SteamPlatform};
use kuro_protocol::api::server::{
    FileContent, PortProtocol, PublishedPort, ReadySpec, StopSignal, StopSpec,
};
use serde_json::{Map, json};
use uuid::Uuid;

use crate::support::{Variant, official};

const SERVER_ID: &str = "0192f3a4-1111-7000-8000-00000000abcd";

fn deployment<'a>(
    settings: &'a kuro_prefab::settings::Values,
    allocations: &'a BTreeMap<String, (String, u16)>,
) -> Deployment<'a> {
    Deployment {
        server_id: Uuid::parse_str(SERVER_ID).unwrap(),
        server_name: "Survie",
        settings,
        memory_mb: 4096,
        cpu_percent: 200,
        allocations,
        content: BTreeMap::new(),
        content_values: BTreeMap::new(),
        packages: BTreeMap::new(),
    }
}

#[test]
fn minecraft_resolves_into_one_hardened_service() {
    let package = official("minecraft-java");
    let mut settings = apply(
        &package.manifest,
        None,
        json!({ "motd": "Bienvenue", "loader": "fabric" })
            .as_object()
            .unwrap(),
    )
    .unwrap();
    settings.insert("rcon_password".to_owned(), json!("generated-secret"));
    let allocations = BTreeMap::from([("game".to_owned(), ("0.0.0.0".to_owned(), 25570))]);

    let deploy = resolve(&package, &deployment(&settings, &allocations)).unwrap();

    assert_eq!(deploy.services.len(), 1);
    let server = &deploy.services[0];
    assert_eq!(server.name, "server");
    assert_eq!(
        server.image,
        "docker.io/itzg/minecraft-server:2026.10.1-java25"
    );
    assert_eq!(server.user, "1000:1000");
    assert_eq!(server.writable, vec!["/tmp".to_owned()]);
    assert_eq!(server.env["TYPE"], "FABRIC");
    assert_eq!(server.env["MEMORY"], "4096M");
    assert_eq!(server.limits.memory_bytes, 4096 * 1024 * 1024);
    assert_eq!(server.limits.cpu_millis, 2000);

    let game = server.ports.iter().find(|p| p.name == "game").unwrap();
    assert_eq!(game.protocol, PortProtocol::Tcp);
    assert_eq!(
        game.published,
        Some(PublishedPort {
            host_ip: "0.0.0.0".to_owned(),
            host_port: 25570
        })
    );
    let rcon = server.ports.iter().find(|p| p.name == "rcon").unwrap();
    assert_eq!(rcon.published, None, "rcon stays on the private network");

    let properties = deploy
        .files
        .iter()
        .find(|f| f.path == "/data/server.properties")
        .unwrap();
    let FileContent::ManagedKeys { keys } = &properties.content else {
        panic!("server.properties should be managed-keys");
    };
    assert_eq!(keys["motd"], "Bienvenue");
    assert_eq!(keys["server-port"], "25565");
    assert_eq!(keys["max-players"], "20");
    assert_eq!(keys["rcon.password"], "generated-secret");

    assert_eq!(deploy.lifecycle.service, "server");
    assert!(
        matches!(&deploy.lifecycle.ready, Some(ReadySpec::Console(p)) if p.starts_with("Done"))
    );
    assert_eq!(deploy.lifecycle.stop, StopSpec::Console("stop".to_owned()));
    assert_eq!(deploy.lifecycle.stop_timeout_secs, 60);
}

#[test]
fn wordpress_splits_resources_between_its_services() {
    let package = official("wordpress");
    let mut settings = apply(&package.manifest, None, &Map::new()).unwrap();
    settings.insert("db_password".to_owned(), json!("shared-secret"));
    let allocations = BTreeMap::from([("http".to_owned(), ("0.0.0.0".to_owned(), 8081))]);

    let deploy = resolve(&package, &deployment(&settings, &allocations)).unwrap();

    let web = deploy.services.iter().find(|s| s.name == "web").unwrap();
    let db = deploy.services.iter().find(|s| s.name == "db").unwrap();
    assert_eq!(web.limits.memory_bytes, 4096 * 1024 * 1024 * 60 / 100);
    assert_eq!(db.limits.memory_bytes, 4096 * 1024 * 1024 * 40 / 100);
    assert_eq!(web.limits.cpu_millis, 1200);
    assert_eq!(db.limits.cpu_millis, 800);
    assert_eq!(web.depends_on, vec!["db".to_owned()]);
    assert_eq!(web.env["WORDPRESS_DB_PASSWORD"], "shared-secret");
    assert_eq!(db.env["MARIADB_PASSWORD"], "shared-secret");
    assert_eq!(web.env["WORDPRESS_DB_HOST"], "db");
    assert!(db.ports.is_empty(), "the database is never published");
    assert!(db.healthcheck.is_some());
    assert_eq!(deploy.lifecycle.stop, StopSpec::Signal(StopSignal::Term));
    assert_eq!(deploy.lifecycle.ready, None);
}

#[test]
fn a_public_port_needs_an_allocation() {
    let package = official("minecraft-java");
    let mut settings = apply(&package.manifest, None, &Map::new()).unwrap();
    settings.insert("rcon_password".to_owned(), json!("x"));
    let none = BTreeMap::new();

    let error = resolve(&package, &deployment(&settings, &none)).unwrap_err();

    assert!(matches!(error, ResolveError::MissingAllocation(port) if port == "game"));
}

#[test]
fn install_steps_are_resolved_with_their_templates() {
    let package = official("ark-survival-ascended");
    let mut settings = apply(&package.manifest, None, &Map::new()).unwrap();
    settings.insert("admin_password".to_owned(), json!("x"));
    let allocations = BTreeMap::from([
        ("game".to_owned(), ("0.0.0.0".to_owned(), 7777)),
        ("peer".to_owned(), ("0.0.0.0".to_owned(), 7778)),
    ]);
    let deployment = deployment(&settings, &allocations);

    // Le déploiement n'est plus refusé : l'installation se résout à part.
    resolve(&package, &deployment).unwrap();
    let install = resolve_install(&package, &deployment).unwrap().unwrap();

    assert_eq!(install.service, "server");
    assert_eq!(install.image, package.manifest.services["server"].image);
    // Serveur Windows : SteamCMD force la plateforme (ADR-0032).
    assert!(matches!(
        &install.steps[0],
        InstallStep::Steamcmd { app: 2_430_930, dir, validate: true, platform: SteamPlatform::Windows, .. } if dir == "/data/server"
    ));
    let InstallStep::Copy { to: script, .. } = &install.steps[1] else {
        panic!("expected the start script");
    };
    assert_eq!(script, "/data/kuro-start.sh");
    let InstallStep::Copy { content, to } = &install.steps[2] else {
        panic!("expected a copy step");
    };
    assert!(to.ends_with("Game.ini"));
    assert!(!content.contains("{{"), "template not rendered: {content}");
}

#[test]
fn a_prefab_without_install_steps_installs_itself() {
    let package = official("minecraft-java");
    let mut settings = apply(&package.manifest, None, &Map::new()).unwrap();
    settings.insert("rcon_password".to_owned(), json!("x"));
    let allocations = BTreeMap::from([("game".to_owned(), ("0.0.0.0".to_owned(), 25565))]);

    let install = resolve_install(&package, &deployment(&settings, &allocations)).unwrap();

    assert_eq!(install, None);
}

#[test]
fn terraria_downloads_extracts_then_cleans_up() {
    let package = official("terraria");
    let settings = apply(&package.manifest, None, &Map::new()).unwrap();
    let allocations = BTreeMap::from([("game".to_owned(), ("0.0.0.0".to_owned(), 7777))]);

    let install = resolve_install(&package, &deployment(&settings, &allocations))
        .unwrap()
        .unwrap();

    assert_eq!(
        install.image,
        "docker.io/library/debian:bookworm-20261020-slim"
    );
    assert!(matches!(
        &install.steps[..],
        [
            InstallStep::Download { url, sha256, to },
            InstallStep::Extract { from, subdir: Some(subdir), executable, .. },
            InstallStep::Remove { path },
        ] if url.starts_with("https://terraria.org/")
            && sha256.len() == 64
            && from == to
            && path == to
            && subdir == "1449/Linux"
            && executable == &["TerrariaServer.bin.x86_64".to_owned()]
    ));
}

#[test]
fn content_slots_render_the_loader_and_game_version_of_the_server() {
    let package = official("minecraft-java");
    let settings = apply(
        &package.manifest,
        None,
        json!({ "loader": "fabric", "version": "1.21.4" })
            .as_object()
            .unwrap(),
    )
    .unwrap();
    let allocations = BTreeMap::from([("game".to_owned(), ("0.0.0.0".to_owned(), 25565))]);

    let plans =
        kuro_prefab::resolve_content(&package, &deployment(&settings, &allocations)).unwrap();

    let mods = plans.iter().find(|p| p.slot == "mods").unwrap();
    assert_eq!(mods.service, "server");
    assert_eq!(mods.dir.as_deref(), Some("/data/mods"));
    let modrinth = mods
        .providers
        .iter()
        .find(|p| p.provider == kuro_prefab::manifest::Provider::Modrinth)
        .unwrap();
    assert_eq!(modrinth.loader.as_deref(), Some("fabric"));
    assert_eq!(modrinth.game_version.as_deref(), Some("1.21.4"));
    let plugins = plans.iter().find(|p| p.slot == "plugins").unwrap();
    assert_eq!(plugins.providers[0].loader.as_deref(), Some("paper"));
}

/// Valheim, dont l'étape `steamcmd` exige le compte de l'installation.
fn valheim_with_account() -> kuro_prefab::Package {
    Variant::of("valheim", |text| {
        text.replace(
            "steamcmd: { app: 896660, dir: /home/container, validate: true }",
            "steamcmd: { app: 896660, dir: /home/container, validate: true, login: account }",
        )
    })
    .load()
    .unwrap()
}

fn valheim_install(package: &kuro_prefab::Package) -> kuro_protocol::api::install::InstallSpec {
    let given = Map::from_iter([("password".to_owned(), json!("valheim-pass"))]);
    let settings = apply(&package.manifest, None, &given).unwrap();
    let allocations = BTreeMap::from([
        ("game".to_owned(), ("0.0.0.0".to_owned(), 2456)),
        ("query".to_owned(), ("0.0.0.0".to_owned(), 2457)),
    ]);
    resolve_install(package, &deployment(&settings, &allocations))
        .unwrap()
        .unwrap()
}

#[test]
fn only_a_step_that_asks_for_the_account_receives_it() {
    let account = SteamLogin {
        username: "kuro_servers".to_owned(),
        password: "pass".to_owned(),
    };
    let package = valheim_with_account();
    assert!(package.manifest.needs_steam_account());
    let mut install = valheim_install(&package);
    // Résolu sans compte : le prefab ne le voit jamais.
    assert!(matches!(
        &install.steps[0],
        InstallStep::Steamcmd { login: None, .. }
    ));

    with_steam_login(&package, &mut install, Some(&account)).unwrap();
    assert!(
        matches!(&install.steps[0], InstallStep::Steamcmd { login: Some(l), .. } if *l == account)
    );

    // Prefab anonyme : rien n'est envoyé, même avec un compte réglé.
    let anonymous = official("valheim");
    assert!(!anonymous.manifest.needs_steam_account());
    let mut install = valheim_install(&anonymous);
    with_steam_login(&anonymous, &mut install, Some(&account)).unwrap();
    assert!(matches!(
        &install.steps[0],
        InstallStep::Steamcmd { login: None, .. }
    ));
}

#[test]
fn a_step_that_asks_for_the_account_fails_without_one() {
    let package = valheim_with_account();
    let mut install = valheim_install(&package);

    assert!(matches!(
        with_steam_login(&package, &mut install, None),
        Err(SteamAccountRequired)
    ));
}

#[test]
fn a_tcp_and_udp_port_is_published_twice_on_one_allocation() {
    let package = Variant::of("valheim", |text| {
        text.replace(
            "game: { container: 2456, protocol: udp, public: true }",
            "game: { container: 2456, protocol: tcp+udp, public: true }",
        )
    })
    .load()
    .unwrap();
    let given = Map::from_iter([("password".to_owned(), json!("valheim-pass"))]);
    let settings = apply(&package.manifest, None, &given).unwrap();
    let allocations = BTreeMap::from([
        ("game".to_owned(), ("0.0.0.0".to_owned(), 30_120)),
        ("query".to_owned(), ("0.0.0.0".to_owned(), 30_121)),
    ]);

    let deploy = resolve(&package, &deployment(&settings, &allocations)).unwrap();

    let game: Vec<_> = deploy.services[0]
        .ports
        .iter()
        .filter(|port| port.name == "game")
        .map(|port| (port.protocol, port.container_port, port.published.clone()))
        .collect();
    let published = Some(PublishedPort {
        host_ip: "0.0.0.0".to_owned(),
        host_port: 30_120,
    });
    assert_eq!(
        game,
        [
            (PortProtocol::Tcp, 2456, published.clone()),
            (PortProtocol::Udp, 2456, published),
        ]
    );
}

#[test]
fn active_packs_are_registered_in_the_world_by_type() {
    let package = official("minecraft-bedrock");
    let settings = apply(&package.manifest, None, &Map::new()).unwrap();
    let allocations = BTreeMap::from([("game".to_owned(), ("0.0.0.0".to_owned(), 19132))]);
    let mut deployment = deployment(&settings, &allocations);
    deployment.packages = BTreeMap::from([(
        "addons".to_owned(),
        vec![
            json!({ "header": { "uuid": "b-1", "version": [1, 0, 0] }, "modules": [{ "type": "data" }] }),
            json!({ "header": { "uuid": "r-1", "version": [1, 2, 0] }, "modules": [{ "type": "resources" }] }),
        ],
    )]);

    let deploy = resolve(&package, &deployment).unwrap();

    let file = |name: &str| {
        deploy
            .files
            .iter()
            .filter(|f| f.path == format!("/data/worlds/Bedrock level/{name}"))
            .map(|f| match &f.content {
                FileContent::Template { content } => {
                    serde_json::from_str::<serde_json::Value>(content).unwrap()
                }
                other => panic!("expected a template, got {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    // Deux déclarations visent les packs de comportement (`data`, `script`) : un seul fichier.
    assert_eq!(
        file("world_behavior_packs.json"),
        [json!([{ "pack_id": "b-1", "version": [1, 0, 0] }])]
    );
    assert_eq!(
        file("world_resource_packs.json"),
        [json!([{ "pack_id": "r-1", "version": [1, 2, 0] }])]
    );
}
