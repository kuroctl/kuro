//! Installation (ADR-0017) : téléchargement vérifié, extraction confinée, réinitialisation,
//! étape en conteneur, première installation au déploiement.

use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use futures_util::StreamExt;
use kuro_daemon::docker::{ConsoleFrom, ContainerFile, Docker, Installer};
use kuro_daemon::fs::ServerDir;
use kuro_daemon::install::download::{DownloadPolicy, is_public};
use kuro_daemon::server::Manager;
use kuro_protocol::api::install::{InstallMode, InstallSpec, InstallStep};
use kuro_protocol::daemon::server::ServerState;
use tempfile::TempDir;
use uuid::Uuid;

use super::https::{Route, TestServer, serve};
use super::support::{IMAGE, single, wait_for};

struct Node {
    manager: Manager,
    events: kuro_daemon::server::StateReceiver,
    data: TempDir,
    id: Uuid,
}

impl Node {
    /// Serveur déployé (spécification et volumes) sans conteneur, avec la politique donnée.
    fn new(policy: DownloadPolicy) -> Self {
        let data = TempDir::new().unwrap();
        let docker = Docker::connect(IMAGE).unwrap();
        let (manager, outgoing) =
            Manager::with_downloads(docker, data.path().to_path_buf(), policy);
        let id = Uuid::now_v7();
        let dir = ServerDir::new(data.path(), id);
        let spec = single(id);
        dir.save_spec(&spec).unwrap();
        // Comme au déploiement : le volume appartient à l'utilisateur du service (1000).
        for (_, volume) in dir.ensure_volumes(&spec).unwrap() {
            #[cfg(unix)]
            let _ = std::os::unix::fs::chown(&volume, Some(1000), Some(1000));
            #[cfg(not(unix))]
            let _ = volume;
        }
        Self {
            manager,
            events: outgoing.events,
            data,
            id,
        }
    }

    fn host(&self, relative: &str) -> std::path::PathBuf {
        self.data
            .path()
            .join(self.id.to_string())
            .join("volumes/app/data")
            .join(relative)
    }

    /// Réinstalle avec ces étapes et attend la fin : `Ok` si `offline`, sinon la cause.
    async fn install(&mut self, mode: InstallMode, steps: Vec<InstallStep>) -> Result<(), String> {
        let spec = InstallSpec {
            service: "app".to_owned(),
            image: IMAGE.to_owned(),
            steps,
        };
        self.manager
            .install(self.id, mode, Some(spec))
            .await
            .map_err(|e| e.to_string())?;
        self.outcome().await
    }

    async fn outcome(&mut self) -> Result<(), String> {
        let deadline = std::time::Duration::from_secs(120);
        tokio::time::timeout(deadline, async {
            while let Some(report) = self.events.recv().await {
                match report.state {
                    ServerState::Offline => return Ok(()),
                    ServerState::Failed => return Err(report.reason.unwrap_or_default()),
                    _ => {}
                }
            }
            Err("event channel closed".to_owned())
        })
        .await
        .unwrap_or_else(|_| Err("timed out".to_owned()))
    }
}

fn sha256(bytes: &[u8]) -> String {
    kuro_runtime::tls::sha256_hex(bytes)
}

/// Archive zip comme celle de Terraria : un dossier racine, sans droits Unix.
fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buffer);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, content) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap();
    buffer.into_inner()
}

fn tar_gz(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    build(&mut builder);
    let tar = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

fn tar_xz(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    build(&mut builder);
    let tar = builder.into_inner().unwrap();
    let mut xz =
        lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::with_preset(1)).unwrap();
    xz.write_all(&tar).unwrap();
    xz.finish().unwrap()
}

/// Téléchargement de `path`, avec l'empreinte de `bytes` (le contenu attendu).
fn download(server: &TestServer, path: &str, bytes: &[u8], to: &str) -> InstallStep {
    InstallStep::Download {
        url: server.url(path),
        sha256: sha256(bytes),
        to: to.to_owned(),
    }
}

#[tokio::test]
async fn a_server_is_downloaded_extracted_and_cleaned_up() {
    let archive = zip(&[
        ("1449/Linux/TerrariaServer.bin.x86_64", b"\x7fELF binary"),
        ("1449/Linux/lib/FNA.dll", b"dll"),
        ("1449/Windows/TerrariaServer.exe", b"MZ"),
    ]);
    let server = serve(HashMap::from([
        ("/server.zip".to_owned(), Route::File(archive.clone())),
        (
            "/latest".to_owned(),
            Route::Redirect("/server.zip".to_owned()),
        ),
    ]))
    .await;
    let mut node = Node::new(server.policy.clone());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![
                // Redirection suivie, comme les liens « latest » des éditeurs.
                InstallStep::Download {
                    url: server.url("/latest"),
                    sha256: sha256(&archive),
                    to: "/data/server.zip".to_owned(),
                },
                InstallStep::Extract {
                    from: "/data/server.zip".to_owned(),
                    to: "/data/server".to_owned(),
                    subdir: Some("1449/Linux".to_owned()),
                    executable: vec!["TerrariaServer.bin.x86_64".to_owned()],
                },
                InstallStep::Copy {
                    content: "maxplayers=8\n".to_owned(),
                    to: "/data/config/serverconfig.txt".to_owned(),
                },
                InstallStep::Remove {
                    path: "/data/server.zip".to_owned(),
                },
            ],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert_eq!(
        std::fs::read(node.host("server/TerrariaServer.bin.x86_64")).unwrap(),
        b"\x7fELF binary"
    );
    assert!(node.host("server/lib/FNA.dll").exists());
    assert!(
        !node.host("server/TerrariaServer.exe").exists(),
        "outside of subdir"
    );
    assert!(!node.host("Windows").exists());
    assert!(
        !node.host("server.zip").exists(),
        "removed after extraction"
    );
    assert_eq!(
        std::fs::read_to_string(node.host("config/serverconfig.txt")).unwrap(),
        "maxplayers=8\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &str| {
            std::fs::metadata(node.host(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("server/TerrariaServer.bin.x86_64"), 0o755);
        assert_eq!(mode("server/lib/FNA.dll"), 0o644);
    }
}

#[tokio::test]
async fn a_wrong_checksum_writes_nothing() {
    let server = serve(HashMap::from([(
        "/server.zip".to_owned(),
        Route::File(b"tampered".to_vec()),
    )]))
    .await;
    let mut node = Node::new(server.policy.clone());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![download(
                &server,
                "/server.zip",
                b"expected",
                "/data/server.zip",
            )],
        )
        .await;

    let reason = outcome.unwrap_err();
    assert!(reason.contains("checksum mismatch"), "{reason}");
    assert!(!node.host("server.zip").exists());
    let leftovers: Vec<_> = std::fs::read_dir(node.host("")).unwrap().collect();
    assert!(leftovers.is_empty(), "temporary file left: {leftovers:?}");
}

#[tokio::test]
async fn internal_addresses_are_never_fetched() {
    let server = serve(HashMap::from([(
        "/server.zip".to_owned(),
        Route::File(b"x".to_vec()),
    )]))
    .await;
    // Politique de production : même un certificat de confiance ne rend pas 127.0.0.1 joignable.
    let policy = DownloadPolicy {
        allow_private: false,
        ..server.policy.clone()
    };
    let mut node = Node::new(policy);

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![download(&server, "/server.zip", b"x", "/data/server.zip")],
        )
        .await;

    let reason = outcome.unwrap_err();
    assert!(reason.contains("non-public address"), "{reason}");
}

#[tokio::test]
async fn plain_http_is_refused() {
    let mut node = Node::new(DownloadPolicy::default());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![InstallStep::Download {
                url: "http://example.com/server.zip".to_owned(),
                sha256: sha256(b"x"),
                to: "/data/server.zip".to_owned(),
            }],
        )
        .await;

    let reason = outcome.unwrap_err();
    assert!(reason.contains("only https"), "{reason}");
}

#[tokio::test]
async fn an_escaping_archive_entry_is_refused() {
    let archive = zip(&[("ok.txt", b"ok"), ("../../escaped.txt", b"pwned")]);
    let server = serve(HashMap::from([(
        "/evil.zip".to_owned(),
        Route::File(archive.clone()),
    )]))
    .await;
    let mut node = Node::new(server.policy.clone());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![
                download(&server, "/evil.zip", &archive, "/data/evil.zip"),
                InstallStep::Extract {
                    from: "/data/evil.zip".to_owned(),
                    to: "/data/out".to_owned(),
                    subdir: None,
                    executable: Vec::new(),
                },
            ],
        )
        .await;

    let reason = outcome.unwrap_err();
    assert!(reason.contains("unsafe archive entry"), "{reason}");
    assert!(
        !node
            .data
            .path()
            .join(node.id.to_string())
            .join("volumes/escaped.txt")
            .exists()
    );
    assert!(!node.host("../escaped.txt").exists());
}

#[tokio::test]
async fn tar_links_are_never_extracted() {
    let archive = tar_gz(|builder| {
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "bin/run.sh", &b"echo\n"[..])
            .unwrap();
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_cksum();
        builder
            .append_link(&mut link, "bin/passwd", "/etc/passwd")
            .unwrap();
    });
    let server = serve(HashMap::from([(
        "/pack.tar.gz".to_owned(),
        Route::File(archive.clone()),
    )]))
    .await;
    let mut node = Node::new(server.policy.clone());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![
                download(&server, "/pack.tar.gz", &archive, "/data/pack.tar.gz"),
                InstallStep::Extract {
                    from: "/data/pack.tar.gz".to_owned(),
                    to: "/data".to_owned(),
                    subdir: None,
                    executable: Vec::new(),
                },
            ],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert_eq!(
        std::fs::read_to_string(node.host("bin/run.sh")).unwrap(),
        "echo\n"
    );
    assert!(std::fs::symlink_metadata(node.host("bin/passwd")).is_err());
}

#[tokio::test]
async fn a_reset_clears_the_volumes_first() {
    let mut node = Node::new(DownloadPolicy::default());
    std::fs::create_dir_all(node.host("world")).unwrap();
    std::fs::write(node.host("world/level.dat"), "old world").unwrap();

    let outcome = node
        .install(
            InstallMode::Reset,
            vec![InstallStep::Copy {
                content: "fresh\n".to_owned(),
                to: "/data/config.txt".to_owned(),
            }],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert!(!node.host("world").exists());
    assert_eq!(
        std::fs::read_to_string(node.host("config.txt")).unwrap(),
        "fresh\n"
    );
}

#[tokio::test]
async fn a_reinstall_keeps_existing_data() {
    let mut node = Node::new(DownloadPolicy::default());
    std::fs::create_dir_all(node.host("world")).unwrap();
    std::fs::write(node.host("world/level.dat"), "my world").unwrap();

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![InstallStep::Copy {
                content: "updated\n".to_owned(),
                to: "/data/config.txt".to_owned(),
            }],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert_eq!(
        std::fs::read_to_string(node.host("world/level.dat")).unwrap(),
        "my world"
    );
}

#[tokio::test]
async fn a_script_step_runs_in_an_isolated_container() {
    let mut node = Node::new(DownloadPolicy::default());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![InstallStep::Script {
                run: "id -u > /data/whoami && echo installed > /data/marker".to_owned(),
                image: None,
            }],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert_eq!(
        std::fs::read_to_string(node.host("marker")).unwrap(),
        "installed\n"
    );
    // Utilisateur du service, jamais root.
    assert_eq!(
        std::fs::read_to_string(node.host("whoami")).unwrap(),
        "1000\n"
    );
}

/// Image dont le point d'entrée est un binaire (`curl`) : l'étape `script` doit tout de même
/// tourner dans un shell (ADR-0039).
const ENTRYPOINT_IMAGE: &str = "docker.io/curlimages/curl:8.14.1";

#[tokio::test]
async fn a_script_step_ignores_the_entrypoint_of_its_image() {
    let mut node = Node::new(DownloadPolicy::default());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![InstallStep::Script {
                run: "echo installed > /data/marker".to_owned(),
                image: Some(ENTRYPOINT_IMAGE.to_owned()),
            }],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert_eq!(
        std::fs::read_to_string(node.host("marker")).unwrap(),
        "installed\n"
    );
}

#[tokio::test]
async fn a_failing_script_fails_the_install() {
    let mut node = Node::new(DownloadPolicy::default());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![InstallStep::Script {
                run: "echo boom; exit 3".to_owned(),
                image: None,
            }],
        )
        .await;

    let reason = outcome.unwrap_err();
    assert!(reason.contains("exited with code 3"), "{reason}");
}

#[tokio::test]
async fn a_path_outside_the_volumes_is_refused_before_running() {
    let node = Node::new(DownloadPolicy::default());
    let spec = InstallSpec {
        service: "app".to_owned(),
        image: IMAGE.to_owned(),
        steps: vec![InstallStep::Remove {
            path: "/etc/passwd".to_owned(),
        }],
    };

    let refused = node
        .manager
        .install(node.id, InstallMode::Reinstall, Some(spec))
        .await;

    assert!(refused.is_err());
}

#[tokio::test]
async fn the_first_install_runs_right_after_the_deployment() {
    let mut node = super::support::node();
    let id = Uuid::now_v7();
    let mut spec = single(id);
    spec.install = Some(InstallSpec {
        service: "app".to_owned(),
        image: IMAGE.to_owned(),
        steps: vec![InstallStep::Copy {
            content: "installed\n".to_owned(),
            to: "/data/marker".to_owned(),
        }],
    });

    node.manager.deploy(spec).unwrap();
    wait_for(&mut node.events, id, ServerState::Installing).await;
    wait_for(&mut node.events, id, ServerState::Offline).await;

    let marker = node
        .data
        .path()
        .join(id.to_string())
        .join("volumes/app/data/marker");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "installed\n");
    node.manager.delete(id).unwrap();
    wait_for(&mut node.events, id, ServerState::Deleted).await;
}

#[test]
fn only_public_addresses_are_reachable() {
    for ip in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
        IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6("fd00::1".parse().unwrap()),
        IpAddr::V6("fe80::1".parse().unwrap()),
        IpAddr::V6("::ffff:127.0.0.1".parse().unwrap()),
    ] {
        assert!(!is_public(ip), "{ip} should be refused");
    }
    for ip in [
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(104, 18, 0, 1)),
        IpAddr::V6("2606:4700::1111".parse().unwrap()),
    ] {
        assert!(is_public(ip), "{ip} should be allowed");
    }
}

#[tokio::test]
async fn a_reconnection_during_an_install_keeps_it_installing() {
    let mut node = Node::new(DownloadPolicy::default());
    let spec = InstallSpec {
        service: "app".to_owned(),
        image: IMAGE.to_owned(),
        steps: vec![InstallStep::Script {
            run: "sleep 3".to_owned(),
            image: None,
        }],
    };
    node.manager
        .install(node.id, InstallMode::Reinstall, Some(spec))
        .await
        .unwrap();
    wait_for(&mut node.events, node.id, ServerState::Installing).await;

    // Lien rétabli pendant l'étape : le conteneur principal est arrêté, mais l'installation
    // continue.
    node.manager.sync(vec![node.id]);
    let after_sync = tokio::time::timeout(std::time::Duration::from_secs(10), node.events.recv())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(after_sync.state, ServerState::Installing);
    assert_eq!(node.outcome().await, Ok(()));
}

#[tokio::test]
async fn a_file_put_in_an_installer_reaches_only_its_user_and_never_its_arguments() {
    // Script de connexion SteamCMD (ADR-0033) : déposé par l'API Docker avant le démarrage.
    let docker = Docker::connect(IMAGE).unwrap();
    let id = Uuid::now_v7();
    docker.ensure_image(IMAGE).await.unwrap();
    docker.ensure_network(id).await.unwrap();
    let command = vec![
        "sh".to_owned(),
        "-c".to_owned(),
        "stat -c '%a %u:%g' /tmp/login.txt && cat /tmp/login.txt".to_owned(),
    ];
    let name = docker
        .create_installer(&Installer {
            server_id: id,
            image: IMAGE,
            command: command.clone(),
            clear_entrypoint: false,
            env: vec![],
            user: "1000:1000",
            mounts: vec![],
            memory_bytes: 64 * 1024 * 1024,
            cpu_millis: 500,
            pids: 64,
        })
        .await
        .unwrap();
    docker
        .put_file(
            &name,
            &ContainerFile {
                dir: "/tmp",
                name: "login.txt",
                content: b"login \"kuro\" \"s3cret\"\n",
                owner: (1000, 1000),
                mode: 0o400,
            },
        )
        .await
        .unwrap();
    docker.start(&name).await.unwrap();
    let code = docker.wait(&name).await.unwrap();
    let lines: Vec<String> = docker
        .console_lines(&name, ConsoleFrom::Since(0))
        .map(|line| line.text)
        .collect()
        .await;
    let _ = docker.remove_container(&name).await;
    let _ = docker.remove_network(id).await;

    assert_eq!(code, 0, "{lines:?}");
    assert_eq!(lines, ["400 1000:1000", "login \"kuro\" \"s3cret\""]);
    assert!(!command.join(" ").contains("s3cret"));
}

#[tokio::test]
async fn a_tar_xz_archive_is_extracted_like_the_others() {
    // Format des builds FiveM (`fx.tar.xz`) et de bien d'autres serveurs.
    let archive = tar_xz(|builder| {
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(
                &mut header,
                "alpine/opt/cfx-server/FXServer",
                &b"\x7fELF!"[..],
            )
            .unwrap();
    });
    let server = serve(HashMap::from([(
        "/fx.tar.xz".to_owned(),
        Route::File(archive.clone()),
    )]))
    .await;
    let mut node = Node::new(server.policy.clone());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![
                download(&server, "/fx.tar.xz", &archive, "/data/fx.tar.xz"),
                InstallStep::Extract {
                    from: "/data/fx.tar.xz".to_owned(),
                    to: "/data".to_owned(),
                    subdir: None,
                    executable: vec!["alpine/opt/cfx-server/FXServer".to_owned()],
                },
            ],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    assert_eq!(
        std::fs::read(node.host("alpine/opt/cfx-server/FXServer")).unwrap(),
        b"\x7fELF!"
    );
}

#[tokio::test]
async fn a_link_inside_the_archive_becomes_a_copy_of_its_file() {
    // Bibliothèques d'un serveur livré en `.tar.xz` : `libfoo.so.6 → libfoo.so.6.0.33`.
    let archive = tar_xz(|builder| {
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "usr/lib/libfoo.so.6.0.33", &b"lib"[..])
            .unwrap();
        for (name, target) in [
            ("usr/lib/libfoo.so.6", "libfoo.so.6.0.33"),
            // Chaîne de liens, résolue jusqu'au fichier.
            ("usr/lib/libfoo.so", "libfoo.so.6"),
            ("bin/foo", "../usr/lib/libfoo.so"),
            // Sort de la destination, ou absolu : jamais suivi.
            ("escape", "../../../../etc/passwd"),
            ("absolute", "/etc/passwd"),
        ] {
            let mut link = tar::Header::new_gnu();
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_size(0);
            link.set_cksum();
            builder.append_link(&mut link, name, target).unwrap();
        }
    });
    let server = serve(HashMap::from([(
        "/pack.tar.xz".to_owned(),
        Route::File(archive.clone()),
    )]))
    .await;
    let mut node = Node::new(server.policy.clone());

    let outcome = node
        .install(
            InstallMode::Reinstall,
            vec![
                download(&server, "/pack.tar.xz", &archive, "/data/pack.tar.xz"),
                InstallStep::Extract {
                    from: "/data/pack.tar.xz".to_owned(),
                    to: "/data/server".to_owned(),
                    subdir: None,
                    executable: Vec::new(),
                },
            ],
        )
        .await;

    assert_eq!(outcome, Ok(()));
    for copy in [
        "server/usr/lib/libfoo.so.6",
        "server/usr/lib/libfoo.so",
        "server/bin/foo",
    ] {
        let path = node.host(copy);
        assert!(
            !std::fs::symlink_metadata(&path).unwrap().is_symlink(),
            "{copy}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"lib", "{copy}");
    }
    for skipped in ["server/escape", "server/absolute"] {
        assert!(
            std::fs::symlink_metadata(node.host(skipped)).is_err(),
            "{skipped}"
        );
    }
}
