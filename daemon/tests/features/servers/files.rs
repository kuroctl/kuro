//! Fichiers déclarés : modes d'écriture, clés gérées, chemins toujours sous un volume.

use std::collections::BTreeMap;

use kuro_daemon::fs::{FsError, ServerDir, managed};
use kuro_protocol::api::server::{FileContent, FileFormat, FileSpec};
use tempfile::TempDir;
use uuid::Uuid;

use super::support::single;

fn keys(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[test]
fn managed_properties_keep_everything_else() {
    let original = "#Minecraft server properties\nmotd=old\nlevel-name=world\npvp: true\n";

    let edited = managed::properties(
        original,
        &keys(&[("motd", "new"), ("pvp", "false"), ("max-players", "20")]),
    );

    assert_eq!(
        edited,
        "#Minecraft server properties\nmotd=new\nlevel-name=world\npvp=false\nmax-players=20\n"
    );
}

#[test]
fn managed_properties_are_escaped() {
    let edited = managed::properties("", &keys(&[("motd", r"a\b")]));

    assert_eq!(edited, "motd=a\\\\b\n");
}

#[test]
fn managed_ini_keys_go_to_their_section() {
    let original = "[ServerSettings]\r\nDifficultyOffset=0.5\r\nXPMultiplier=1\r\n\r\n[SessionSettings]\r\nSessionName=Old\r\n";

    let edited = managed::ini(
        original,
        &keys(&[
            ("ServerSettings.XPMultiplier", "2"),
            ("ServerSettings.HarvestAmountMultiplier", "3"),
            ("MessageOfTheDay.Message", "Hi"),
        ]),
    );

    assert_eq!(
        edited,
        "[ServerSettings]\r\nDifficultyOffset=0.5\r\nXPMultiplier=2\r\nHarvestAmountMultiplier=3\r\n\r\n\
         [SessionSettings]\r\nSessionName=Old\r\n\r\n[MessageOfTheDay]\r\nMessage=Hi\r\n"
    );
}

fn write(dir: &ServerDir, file: FileSpec) -> Result<Option<std::path::PathBuf>, FsError> {
    let mut spec = single(Uuid::now_v7());
    spec.files = vec![file.clone()];
    dir.ensure_volumes(&spec).unwrap();
    dir.write_file(&spec, &file)
}

fn file(path: &str, content: FileContent) -> FileSpec {
    FileSpec {
        service: "app".to_owned(),
        path: path.to_owned(),
        format: FileFormat::Text,
        content,
    }
}

#[test]
fn a_user_file_is_never_overwritten() {
    let root = TempDir::new().unwrap();
    let dir = ServerDir::new(root.path(), Uuid::now_v7());
    let initial = FileContent::User {
        content: Some("initial".to_owned()),
    };

    let first = write(&dir, file("/data/conf/notes.txt", initial.clone()))
        .unwrap()
        .unwrap();
    std::fs::write(&first, "edited by the admin").unwrap();
    let second = write(&dir, file("/data/conf/notes.txt", initial)).unwrap();

    assert_eq!(second, None);
    assert_eq!(
        std::fs::read_to_string(first).unwrap(),
        "edited by the admin"
    );
}

#[test]
fn a_template_file_is_rewritten_every_time() {
    let root = TempDir::new().unwrap();
    let dir = ServerDir::new(root.path(), Uuid::now_v7());

    let path = write(
        &dir,
        file(
            "/data/motd.txt",
            FileContent::Template {
                content: "v1".to_owned(),
            },
        ),
    )
    .unwrap()
    .unwrap();
    write(
        &dir,
        file(
            "/data/motd.txt",
            FileContent::Template {
                content: "v2".to_owned(),
            },
        ),
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(path).unwrap(), "v2");
}

#[test]
fn paths_never_leave_the_volume() {
    let root = TempDir::new().unwrap();
    let dir = ServerDir::new(root.path(), Uuid::now_v7());
    let spec = single(Uuid::now_v7());

    for path in ["/etc/passwd", "/data/../etc/passwd", "/dataset/x", "/"] {
        assert!(
            matches!(
                dir.resolve(&spec.services[0], path),
                Err(FsError::OutsideVolume(_))
            ),
            "{path} accepted"
        );
    }
    assert!(
        dir.resolve(&spec.services[0], "/data/./world/level.dat")
            .is_ok()
    );
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_cannot_redirect_a_write() {
    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let dir = ServerDir::new(root.path(), Uuid::now_v7());
    let spec = single(Uuid::now_v7());
    dir.ensure_volumes(&spec).unwrap();
    // Un processus du serveur aurait remplacé un dossier par un lien vers l'hôte.
    std::os::unix::fs::symlink(outside.path(), dir.volume_dir("app", "data").join("conf")).unwrap();

    let result = dir.resolve(&spec.services[0], "/data/conf/app.properties");

    assert!(matches!(result, Err(FsError::Symlink(_))));
}

#[test]
fn removing_a_server_deletes_its_data() {
    let root = TempDir::new().unwrap();
    let dir = ServerDir::new(root.path(), Uuid::now_v7());
    let spec = single(Uuid::now_v7());
    dir.ensure_volumes(&spec).unwrap();
    dir.save_spec(&spec).unwrap();

    assert_eq!(dir.load_spec().unwrap(), Some(spec));
    dir.remove().unwrap();
    assert!(!dir.root().exists());
    assert_eq!(dir.load_spec().unwrap(), None);
}
