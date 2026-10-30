//! Résolution : prefab + réglages + allocations + ressources → `server.deploy` (ADR-0013).
//!
//! Le daemon reçoit une spécification complète et ne connaît pas les prefabs. Les ressources du
//! serveur sont réparties entre ses services selon leur part (`resources.share`).

use std::collections::BTreeMap;

use kuro_protocol::api::BackupHook;
use kuro_protocol::api::install::{InstallSpec, InstallStep, SteamLogin};
use kuro_protocol::api::server::{
    FileContent, FileFormat as WireFormat, FileSpec, HealthcheckSpec, LifecycleSpec, Limits,
    PortSpec, PublishedPort, ReadySpec, ServerDeploy, ServiceSpec, StopSignal, StopSpec,
    VolumeSpec, WebRoute,
};
use uuid::Uuid;

use crate::manifest::{FileFormat, FileMode, Ready, Signal, SteamcmdLogin, Step, Stop};
use crate::package::Package;
use crate::settings::Values;
use crate::template::{Context, Template, TemplateError};

/// Utilisateur d'un service qui n'en déclare pas.
pub const DEFAULT_USER: &str = "1000:1000";
/// Processus par service.
pub const PIDS_PER_SERVICE: u32 = 1024;

/// Ce qui décide d'un déploiement, en plus du prefab.
#[derive(Debug, Clone)]
pub struct Deployment<'a> {
    pub server_id: Uuid,
    pub server_name: &'a str,
    /// Réglages, secrets compris.
    pub settings: &'a Values,
    pub memory_mb: u32,
    /// 100 = un cœur.
    pub cpu_percent: u32,
    /// Port public du prefab → allocation (adresse, port de l'hôte).
    pub allocations: &'a BTreeMap<String, (String, u16)>,
    /// `https://<identifiant>.<domaine des sites>` (ADR-0038) ; absente pour un serveur sans
    /// port web. Un prefab web sans adresse ne se résout pas (`server.web_url` sans valeur).
    pub web_url: Option<String>,
    /// Emplacement de contenu → identifiants (mods…).
    pub content: BTreeMap<String, Vec<String>>,
    /// `<emplacement>.<champ>` → valeurs saisies pour les contenus actifs (ADR-0030).
    pub content_values: BTreeMap<String, Vec<String>>,
    /// Emplacement → descriptions des paquets actifs, dans l'ordre d'ajout (mode `extract`) :
    /// les entrées des fichiers `register`.
    pub packages: BTreeMap<String, Vec<serde_json::Value>>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("{at}: {source}")]
    Template {
        at: String,
        #[source]
        source: TemplateError,
    },
    #[error("public port `{0}` has no allocation")]
    MissingAllocation(String),
    #[error("unknown service `{0}`")]
    UnknownService(String),
}

/// Contexte des gabarits : réglages, ports, ressources, identité du serveur.
fn context<'a>(
    package: &Package,
    deployment: &Deployment<'a>,
) -> Result<Context<'a>, ResolveError> {
    let m = &package.manifest;
    let mut ports = BTreeMap::new();
    for service in m.services.values() {
        for (name, port) in &service.ports {
            let host = if port.public {
                let (_, host_port) = deployment
                    .allocations
                    .get(name)
                    .ok_or_else(|| ResolveError::MissingAllocation(name.clone()))?;
                Some(*host_port)
            } else {
                None
            };
            ports.insert(name.clone(), (host, port.container));
        }
    }
    Ok(Context {
        settings: Some(deployment.settings),
        ports,
        memory_mb: deployment.memory_mb,
        cpu_percent: deployment.cpu_percent,
        server_id: deployment.server_id.to_string(),
        server_name: deployment.server_name.to_owned(),
        server_web_url: deployment.web_url.clone(),
        content: deployment.content.clone(),
        content_values: deployment.content_values.clone(),
    })
}

fn renderer<'c>(
    context: &'c Context<'_>,
) -> impl Fn(String, &str) -> Result<String, ResolveError> + 'c {
    move |at: String, source: &str| {
        Template::parse(source)
            .and_then(|template| template.render(context))
            .map_err(|source| ResolveError::Template { at, source })
    }
}

/// Emplacement de contenu d'un serveur, gabarits rendus (ADR-0020).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentPlan {
    pub slot: String,
    pub kind: crate::manifest::ContentKind,
    pub service: String,
    /// Dossier de dépôt dans le conteneur ; absent en mode `reference`.
    pub dir: Option<String>,
    pub providers: Vec<ContentSource>,
    /// Valeurs à saisir pour chaque contenu (ADR-0030).
    pub fields: BTreeMap<String, crate::manifest::ContentField>,
    /// Mode `extract` : description et routes des paquets, prêtes pour `content.install`.
    pub extract: Option<kuro_protocol::api::ContentExtract>,
}

/// Provider d'un emplacement, loader et version du jeu rendus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentSource {
    pub provider: crate::manifest::Provider,
    pub game: Option<String>,
    pub class: Option<String>,
    pub loader: Option<String>,
    pub game_version: Option<String>,
}

/// Emplacements de contenu du prefab, pour les réglages du serveur.
pub fn resolve_content(
    package: &Package,
    deployment: &Deployment<'_>,
) -> Result<Vec<ContentPlan>, ResolveError> {
    let context = context(package, deployment)?;
    let render = renderer(&context);
    let mut plans = Vec::new();
    for (slot, def) in &package.manifest.content {
        let mut providers = Vec::new();
        for (index, decl) in def.providers.iter().enumerate() {
            let at = |field: &str| format!("content.{slot}.providers[{index}].{field}");
            providers.push(ContentSource {
                provider: decl.provider,
                game: decl.game.clone(),
                class: decl.class.clone(),
                loader: decl
                    .loader
                    .as_deref()
                    .map(|s| render(at("loader"), s))
                    .transpose()?,
                game_version: decl
                    .game_version
                    .as_deref()
                    .map(|s| render(at("game_version"), s))
                    .transpose()?,
            });
        }
        let extract =
            (def.install.mode == crate::manifest::ContentInstallMode::Extract).then(|| {
                kuro_protocol::api::ContentExtract {
                    describe: def.install.describe.clone().unwrap_or_default(),
                    routes: def
                        .install
                        .route
                        .iter()
                        .map(|route| kuro_protocol::api::ExtractRoute {
                            value: route.when.value.clone(),
                            is: route.when.is.clone(),
                            dir: route.dir.clone(),
                        })
                        .collect(),
                }
            });
        // Archives d'un emplacement `extract` : dans `dir`, sinon dans le dossier de la 1re route.
        let dir = def.install.dir.clone().or_else(|| {
            extract
                .as_ref()
                .and_then(|e| e.routes.first().map(|r| r.dir.clone()))
        });
        plans.push(ContentPlan {
            fields: def.fields.clone(),
            slot: slot.clone(),
            kind: def.kind,
            service: def.service.clone(),
            dir,
            providers,
            extract,
        });
    }
    Ok(plans)
}

/// Ce que le prefab demande pour une sauvegarde (ADR-0018), prêt pour `backup.create`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupPlan {
    /// Motifs en chemins de volumes : `service/volume/…`.
    pub ignore: Vec<String>,
    pub before: Vec<BackupHook>,
    pub after: Vec<BackupHook>,
}

/// Section `backup` du prefab, chemins du conteneur traduits en chemins de volumes. Un motif qui
/// ne tombe sous aucun volume (refusé à la validation) est ignoré.
pub fn resolve_backup(package: &Package) -> BackupPlan {
    let m = &package.manifest;
    let Some(backup) = &m.backup else {
        return BackupPlan::default();
    };
    let hooks = |list: &[crate::manifest::BackupHook]| {
        list.iter()
            .map(|hook| BackupHook {
                console: hook.console.clone(),
                wait: hook.wait.clone(),
            })
            .collect()
    };
    let ignore = backup
        .ignore
        .iter()
        .flat_map(|(service, patterns)| {
            let volumes = m.services.get(service).map(|s| &s.volumes);
            patterns.iter().filter_map(move |pattern| {
                volumes?.iter().find_map(|(name, volume)| {
                    let root = volume.path.trim_end_matches('/');
                    let rest = if pattern == root {
                        ""
                    } else {
                        pattern.strip_prefix(&format!("{root}/"))?
                    };
                    Some(
                        [service.as_str(), name.as_str(), rest]
                            .iter()
                            .filter(|part| !part.is_empty())
                            .copied()
                            .collect::<Vec<_>>()
                            .join("/"),
                    )
                })
            })
        })
        .collect();
    BackupPlan {
        ignore,
        before: hooks(&backup.before),
        after: hooks(&backup.after),
    }
}

/// Le prefab exige le compte Steam de l'installation, qui n'est pas réglé (ADR-0033).
#[derive(Debug, thiserror::Error)]
#[error("this prefab needs the Steam account of the installation")]
pub struct SteamAccountRequired;

/// Donne le compte Steam de l'installation aux étapes `steamcmd` qui l'exigent (`login:
/// account`), et à elles seules. Les étapes résolues suivent celles du manifest une pour une.
pub fn with_steam_login(
    package: &Package,
    install: &mut InstallSpec,
    account: Option<&SteamLogin>,
) -> Result<(), SteamAccountRequired> {
    let Some(declared) = &package.manifest.install else {
        return Ok(());
    };
    for (step, resolved) in declared.steps.iter().zip(install.steps.iter_mut()) {
        if let (
            Step::Steamcmd {
                login: SteamcmdLogin::Account,
                ..
            },
            InstallStep::Steamcmd { login, .. },
        ) = (step, resolved)
        {
            *login = Some(account.ok_or(SteamAccountRequired)?.clone());
        }
    }
    Ok(())
}

/// Référence `{{ item.<chemin> }}` d'une entrée d'inscription, sinon `None` (texte fixe).
pub fn item_reference(template: &str) -> Option<&str> {
    let inner = template
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim();
    inner.strip_prefix("item.").filter(|path| !path.is_empty())
}

/// Entrée d'un fichier d'inscription pour un paquet : textes fixes, ou valeurs de sa description.
fn register_entry(
    entry: &indexmap::IndexMap<String, String>,
    description: &serde_json::Value,
) -> serde_json::Value {
    let object = entry
        .iter()
        .map(|(key, template)| {
            let value = match item_reference(template) {
                Some(path) => kuro_protocol::api::lookup(description, path)
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
                None => serde_json::Value::String(template.clone()),
            };
            (key.clone(), value)
        })
        .collect();
    serde_json::Value::Object(object)
}

/// Étapes d'installation résolues (ADR-0017), ou `None` si le prefab s'installe lui-même.
pub fn resolve_install(
    package: &Package,
    deployment: &Deployment<'_>,
) -> Result<Option<InstallSpec>, ResolveError> {
    let m = &package.manifest;
    let Some(install) = &m.install else {
        return Ok(None);
    };
    let context = context(package, deployment)?;
    let render = renderer(&context);
    let image = match &install.image {
        Some(image) => image.clone(),
        None => m
            .services
            .get(&install.service)
            .map(|s| s.image.clone())
            .ok_or_else(|| ResolveError::UnknownService(install.service.clone()))?,
    };
    let steps = install
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let at = |field: &str| format!("install.steps[{index}].{field}");
            Ok(match step {
                Step::Download { url, sha256, to } => InstallStep::Download {
                    url: render(at("url"), url)?,
                    sha256: sha256.clone(),
                    to: to.clone(),
                },
                Step::Extract {
                    from,
                    to,
                    subdir,
                    executable,
                } => InstallStep::Extract {
                    from: from.clone(),
                    to: to.clone(),
                    subdir: subdir.clone(),
                    executable: executable.clone(),
                },
                Step::Copy { template, to } => {
                    let source = package.templates.get(template).cloned().unwrap_or_default();
                    InstallStep::Copy {
                        content: render(at("template"), &source)?,
                        to: to.clone(),
                    }
                }
                Step::Remove { path } => InstallStep::Remove { path: path.clone() },
                Step::Steamcmd {
                    app,
                    dir,
                    beta,
                    validate,
                    platform,
                    login: _,
                } => InstallStep::Steamcmd {
                    app: *app,
                    dir: dir.clone(),
                    beta: beta.clone(),
                    validate: *validate,
                    platform: *platform,
                    // Rempli au moment de l'envoi (`with_steam_login`) : jamais dans un prefab.
                    login: None,
                },
                Step::Script { image, run } => InstallStep::Script {
                    run: run.clone(),
                    image: image.clone(),
                },
            })
        })
        .collect::<Result<_, ResolveError>>()?;
    Ok(Some(InstallSpec {
        service: install.service.clone(),
        image,
        steps,
    }))
}

pub fn resolve(
    package: &Package,
    deployment: &Deployment<'_>,
) -> Result<ServerDeploy, ResolveError> {
    let m = &package.manifest;
    let context = context(package, deployment)?;
    let render = renderer(&context);

    let single = m.services.len() == 1;
    let mut services = Vec::with_capacity(m.services.len());
    for (name, s) in &m.services {
        let share = u64::from(s.resources.share.unwrap_or(if single { 100 } else { 1 }));
        let command = s
            .command
            .iter()
            .enumerate()
            .map(|(i, arg)| render(format!("services.{name}.command[{i}]"), arg))
            .collect::<Result<_, _>>()?;
        let env = s
            .env
            .iter()
            .map(|(key, value)| {
                Ok((
                    key.clone(),
                    render(format!("services.{name}.env.{key}"), value)?,
                ))
            })
            .collect::<Result<_, ResolveError>>()?;
        // Un port `tcp+udp` garde une seule allocation, publiée une fois par protocole.
        let ports = s
            .ports
            .iter()
            .flat_map(|(port, decl)| {
                let published = decl
                    .public
                    .then(|| deployment.allocations.get(port))
                    .flatten()
                    .map(|(host_ip, host_port)| PublishedPort {
                        host_ip: host_ip.clone(),
                        host_port: *host_port,
                    });
                decl.protocol.wire().iter().map(move |protocol| PortSpec {
                    name: port.clone(),
                    container_port: decl.container,
                    protocol: *protocol,
                    published: published.clone(),
                })
            })
            .collect();
        let healthcheck = match &s.healthcheck {
            Some(health) => Some(HealthcheckSpec {
                command: health
                    .command
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| {
                        render(format!("services.{name}.healthcheck.command[{i}]"), arg)
                    })
                    .collect::<Result<_, _>>()?,
                interval_secs: health.interval,
                retries: health.retries,
            }),
            None => None,
        };
        services.push(ServiceSpec {
            name: name.clone(),
            image: s.image.clone(),
            command,
            env,
            ports,
            volumes: s
                .volumes
                .iter()
                .map(|(volume, decl)| VolumeSpec {
                    name: volume.clone(),
                    path: decl.path.clone(),
                })
                .collect(),
            writable: s.writable.clone(),
            user: s.user.clone().unwrap_or_else(|| DEFAULT_USER.to_owned()),
            limits: Limits {
                memory_bytes: u64::from(deployment.memory_mb) * 1024 * 1024 * share / 100,
                cpu_millis: u32::try_from(u64::from(deployment.cpu_percent) * 10 * share / 100)
                    .unwrap_or(u32::MAX)
                    .max(1),
                pids: PIDS_PER_SERVICE,
            },
            depends_on: s.depends_on.clone(),
            healthcheck,
        });
    }

    let mut files = Vec::with_capacity(m.files.len());
    for (id, file) in &m.files {
        let template_text =
            |path: &String| package.templates.get(path).cloned().unwrap_or_default();
        let content = match file.mode {
            FileMode::Template => FileContent::Template {
                content: render(
                    format!("files.{id}.template"),
                    &file
                        .template
                        .as_ref()
                        .map(template_text)
                        .unwrap_or_default(),
                )?,
            },
            FileMode::ManagedKeys => FileContent::ManagedKeys {
                keys: file
                    .keys
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            key.clone(),
                            render(format!("files.{id}.keys.{key}"), value)?,
                        ))
                    })
                    .collect::<Result<_, ResolveError>>()?,
            },
            FileMode::User => FileContent::User {
                content: match &file.template {
                    Some(path) => Some(render(
                        format!("files.{id}.template"),
                        &template_text(path),
                    )?),
                    None => None,
                },
            },
        };
        files.push(FileSpec {
            service: file.service.clone(),
            path: file.path.clone(),
            format: wire_format(file.format),
            content,
        });
    }

    // Fichiers d'inscription (ADR-0030) : une liste JSON par fichier, une entrée par paquet
    // actif ; plusieurs déclarations peuvent viser le même fichier (packs `data` et `script`).
    let mut registers: indexmap::IndexMap<(String, String), Vec<serde_json::Value>> =
        indexmap::IndexMap::new();
    for (slot, def) in &m.content {
        let packages = deployment.packages.get(slot).map_or(&[][..], Vec::as_slice);
        for (index, register) in def.register.iter().enumerate() {
            let path = render(
                format!("content.{slot}.register[{index}].file"),
                &register.file,
            )?;
            let entries = registers.entry((def.service.clone(), path)).or_default();
            entries.extend(
                packages
                    .iter()
                    .filter(|description| {
                        register.when.as_ref().is_none_or(|when| {
                            kuro_protocol::api::matches(description, &when.value, &when.is)
                        })
                    })
                    .map(|description| register_entry(&register.entry, description)),
            );
        }
    }
    for ((service, path), entries) in registers {
        files.push(FileSpec {
            service,
            path,
            format: WireFormat::Json,
            content: FileContent::Template {
                content: serde_json::to_string_pretty(&entries).unwrap_or_default(),
            },
        });
    }

    // Site web (ADR-0038) : le port `web` du prefab, servi par le proxy du nœud à l'adresse du
    // serveur. Sans adresse (domaine des sites retiré), aucune route : le site n'est pas servi.
    let web = match (m.web_port(), &deployment.web_url) {
        (Some((service, _, decl)), Some(url)) => vec![WebRoute {
            host: url
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_owned(),
            service: service.to_owned(),
            container_port: decl.container,
        }],
        _ => Vec::new(),
    };
    let lifecycle = &m.lifecycle;
    Ok(ServerDeploy {
        server_id: deployment.server_id,
        services,
        files,
        web,
        // Ajoutée par l'appelant à la création seulement (`resolve_install`).
        install: None,
        // Quota et suspension : propres au serveur, ajoutés par l'appelant (ADR-0019).
        disk_bytes: 0,
        suspended: false,
        defer: false,
        lifecycle: LifecycleSpec {
            service: lifecycle.service.clone(),
            ready: lifecycle.ready.as_ref().map(|ready| match ready {
                Ready::Console(pattern) => ReadySpec::Console(pattern.clone()),
                Ready::Healthcheck(_) => ReadySpec::Healthcheck,
            }),
            stop: match &lifecycle.stop {
                Stop::Console(command) => StopSpec::Console(command.clone()),
                Stop::Signal(signal) => StopSpec::Signal(match signal {
                    Signal::Term => StopSignal::Term,
                    Signal::Int => StopSignal::Int,
                    Signal::Quit => StopSignal::Quit,
                }),
            },
            stop_timeout_secs: lifecycle.timeout,
        },
    })
}

fn wire_format(format: FileFormat) -> WireFormat {
    match format {
        FileFormat::Ini => WireFormat::Ini,
        FileFormat::Properties => WireFormat::Properties,
        FileFormat::Json => WireFormat::Json,
        FileFormat::Yaml => WireFormat::Yaml,
        FileFormat::Toml => WireFormat::Toml,
        FileFormat::Xml => WireFormat::Xml,
        FileFormat::Cfg => WireFormat::Cfg,
        FileFormat::Lua => WireFormat::Lua,
        FileFormat::Php => WireFormat::Php,
        FileFormat::Sh => WireFormat::Sh,
        FileFormat::Text => WireFormat::Text,
    }
}
