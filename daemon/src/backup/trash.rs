//! Corbeille des sauvegardes (ADR-0059) : à la suppression d'un serveur, ses archives sont gardées
//! `keep_backups_days` jours dans `<data_root>/.trash/<id du serveur>/backups/`, puis effacées par
//! le passage périodique du nœud. Un autre serveur du nœud peut les reprendre (`backup.adopt`).
//! Le dossier `.trash` n'est pas un identifiant de serveur : la liste des serveurs l'ignore.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

const TRASH: &str = ".trash";
/// Date d'effacement (secondes Unix), écrite dans le dossier gardé.
const PURGE_AFTER: &str = "purge_after";

fn dir(data_root: &Path, server_id: Uuid) -> PathBuf {
    data_root.join(TRASH).join(server_id.to_string())
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Garde les archives du serveur (`backups`, dans son dossier) ; rien sans archive ou sans
/// rétention. Un simple renommage : le dossier reste sur le même disque.
pub fn keep(
    data_root: &Path,
    server_id: Uuid,
    backups: &Path,
    days: u32,
    now: u64,
) -> std::io::Result<()> {
    let empty = std::fs::read_dir(backups).map_or(true, |mut entries| entries.next().is_none());
    if days == 0 || empty {
        return Ok(());
    }
    let kept = dir(data_root, server_id);
    super::create_private_dir(&kept)?;
    std::fs::rename(backups, kept.join("backups"))?;
    let purge_after = now + u64::from(days) * 86_400;
    std::fs::write(kept.join(PURGE_AFTER), purge_after.to_string())
}

/// Efface les dossiers gardés dont la date est passée ; retourne leur nombre.
pub fn purge_expired(data_root: &Path, now: u64) -> std::io::Result<usize> {
    let Ok(entries) = std::fs::read_dir(data_root.join(TRASH)) else {
        return Ok(0);
    };
    let mut purged = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let due = std::fs::read_to_string(path.join(PURGE_AFTER))
            .ok()
            .and_then(|text| text.trim().parse::<u64>().ok())
            // Sans date lisible : effacé, plutôt que gardé pour toujours.
            .is_none_or(|purge_after| purge_after <= now);
        if due {
            std::fs::remove_dir_all(&path)?;
            purged += 1;
        }
    }
    Ok(purged)
}

/// Déplace les archives gardées de `from` dans le dossier de sauvegardes `backups` d'un serveur
/// du nœud, puis vide la corbeille de `from` ; retourne le nombre de fichiers déplacés.
pub fn adopt(data_root: &Path, from: Uuid, backups: &Path) -> std::io::Result<usize> {
    let kept = dir(data_root, from);
    let source = kept.join("backups");
    let entries = std::fs::read_dir(&source)?;
    super::create_private_dir(backups)?;
    let mut moved = 0;
    for entry in entries.filter_map(Result::ok) {
        std::fs::rename(entry.path(), backups.join(entry.file_name()))?;
        moved += 1;
    }
    std::fs::remove_dir_all(&kept)?;
    Ok(moved)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn server_with_backup(root: &Path, id: Uuid) -> PathBuf {
        let backups = root.join(id.to_string()).join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("b1.tar.zst"), b"archive").unwrap();
        std::fs::write(backups.join("b1.json"), b"{}").unwrap();
        backups
    }

    #[test]
    fn backups_are_kept_then_purged_once_their_date_has_passed() {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let backups = server_with_backup(root.path(), id);

        keep(root.path(), id, &backups, 7, 1_000).unwrap();
        assert!(!backups.exists());
        assert!(dir(root.path(), id).join("backups/b1.tar.zst").is_file());

        assert_eq!(purge_expired(root.path(), 1_000 + 6 * 86_400).unwrap(), 0);
        assert_eq!(purge_expired(root.path(), 1_000 + 7 * 86_400).unwrap(), 1);
        assert!(!dir(root.path(), id).exists());
    }

    #[test]
    fn nothing_is_kept_without_retention_or_backup() {
        let root = tempfile::tempdir().unwrap();
        let id = Uuid::now_v7();
        let backups = server_with_backup(root.path(), id);
        keep(root.path(), id, &backups, 0, 1_000).unwrap();
        assert!(backups.exists());

        let empty = root.path().join("other/backups");
        std::fs::create_dir_all(&empty).unwrap();
        keep(root.path(), Uuid::now_v7(), &empty, 7, 1_000).unwrap();
        assert!(!root.path().join(TRASH).exists());
    }

    #[test]
    fn kept_backups_are_adopted_by_another_server() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (Uuid::now_v7(), Uuid::now_v7());
        let backups = server_with_backup(root.path(), old);
        keep(root.path(), old, &backups, 7, 1_000).unwrap();

        let target = root.path().join(new.to_string()).join("backups");
        assert_eq!(adopt(root.path(), old, &target).unwrap(), 2);
        assert!(target.join("b1.tar.zst").is_file());
        assert!(!dir(root.path(), old).exists());
        assert!(adopt(root.path(), old, &target).is_err());
    }
}
