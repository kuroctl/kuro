//! Écriture des fichiers de configuration et des secrets (clés privées).

use std::fs;
use std::io;
use std::path::Path;

/// Écrit `content`, en créant les dossiers parents.
pub fn write_file(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Comme [`write_file`], le fichier n'étant lisible que par son propriétaire sous Unix.
pub fn write_secret(path: &Path, content: &str) -> io::Result<()> {
    write_file(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
