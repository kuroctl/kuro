//! Spécifications déployées : écriture atomique, et listing d'un dossier borné.

use kuro_daemon::fs::ServerDir;
use kuro_protocol::daemon::FileKind;
use uuid::Uuid;

use super::support::{node, single};

#[test]
fn a_saved_spec_leaves_no_temporary_file_and_reloads_identically() {
    let data = tempfile::TempDir::new().unwrap();
    let id = Uuid::now_v7();
    let dir = ServerDir::new(data.path(), id);
    let spec = single(id);

    dir.save_spec(&spec).unwrap();
    dir.save_spec(&spec).unwrap();

    let kuro = data.path().join(id.to_string()).join(".kuro");
    let names: Vec<String> = std::fs::read_dir(&kuro)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["spec.json"]);
    assert_eq!(dir.load_spec().unwrap(), Some(spec));
}

#[tokio::test]
async fn a_huge_directory_is_listed_truncated() {
    let node = node();
    let id = Uuid::now_v7();
    let dir = ServerDir::new(node.data.path(), id);
    dir.save_spec(&single(id)).unwrap();
    let volume = dir.volume_dir("app", "data");
    std::fs::create_dir_all(&volume).unwrap();
    for i in 0..5_010 {
        std::fs::write(volume.join(format!("file-{i:05}.txt")), b"").unwrap();
    }

    let listing = node.manager.file_list(id, "/app/data").await.unwrap();

    assert!(listing.truncated);
    assert_eq!(listing.entries.len(), 5_000);
    assert!(listing.entries.iter().all(|e| e.kind == FileKind::File));
    assert_eq!(listing.entries[0].name, "file-00000.txt");

    // Un petit dossier n'est pas marqué tronqué.
    let small = dir.volume_dir("app", "data").join("sub");
    std::fs::create_dir_all(&small).unwrap();
    std::fs::write(small.join("one.txt"), b"").unwrap();
    let listing = node.manager.file_list(id, "/app/data/sub").await.unwrap();
    assert!(!listing.truncated);
    assert_eq!(listing.entries.len(), 1);
}
