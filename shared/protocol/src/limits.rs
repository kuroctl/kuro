//! Limites et délais du lien API ↔ daemon.

use std::time::Duration;

/// Période d'envoi du `heartbeat` par le daemon.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// Sans `heartbeat` pendant ce délai, l'API passe le nœud `offline`.
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);

/// Taille maximale d'un message encodé (1 MiB).
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Lignes de console renvoyées au début d'un `server.watch`, et gardées par l'API pour les
/// spectateurs suivants.
pub const CONSOLE_HISTORY_LINES: usize = 200;

/// Taille maximale d'une ligne de console : au-delà, elle est tronquée.
pub const MAX_CONSOLE_LINE_BYTES: usize = 4096;

/// Lignes maximales par message `console.line`.
pub const MAX_CONSOLE_BATCH_LINES: usize = 128;

/// Caractères maximaux d'une commande console.
pub const MAX_COMMAND_CHARS: usize = 1024;

/// Commande console acceptable : non vide après espaces, une seule ligne, aucun caractère de
/// contrôle, au plus [`MAX_COMMAND_CHARS`] caractères. Vérifiée par l'API et par le daemon.
pub fn is_valid_command(line: &str) -> bool {
    !line.trim().is_empty()
        && line.chars().count() <= MAX_COMMAND_CHARS
        && !line.chars().any(char::is_control)
}

/// Tronque une ligne de console à [`MAX_CONSOLE_LINE_BYTES`], sur une frontière de caractère.
pub fn truncate_console_line(mut line: String) -> String {
    if line.len() > MAX_CONSOLE_LINE_BYTES {
        let mut end = MAX_CONSOLE_LINE_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

/// Taille maximale d'un fichier lu ou écrit comme texte.
pub const MAX_TEXT_FILE_BYTES: usize = 512 * 1024;

/// Taille maximale d'un segment de chemin (nom de fichier).
pub const MAX_PATH_SEGMENT_BYTES: usize = 255;

/// Segments d'un chemin de fichier (`/<service>/<volume>/…`), ou `None` s'il est refusé : `..`,
/// `.`, segment vide (sauf la barre initiale et finale), `\`, octet nul ou caractère de contrôle,
/// segment de plus de [`MAX_PATH_SEGMENT_BYTES`]. Vérifié par l'API et par le daemon.
pub fn path_segments(path: &str) -> Option<Vec<&str>> {
    let trimmed = path.strip_prefix('/').unwrap_or(path);
    if trimmed.starts_with('/') {
        return None;
    }
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    let segments: Vec<&str> = trimmed.split('/').collect();
    segments
        .iter()
        .all(|segment| is_valid_name(segment))
        .then_some(segments)
}

/// Nom d'une entrée de dossier acceptable (voir [`path_segments`]).
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= MAX_PATH_SEGMENT_BYTES
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

/// Octets lus par `backup.read` : environ 700 Ko une fois en base64, sous
/// [`MAX_MESSAGE_BYTES`].
pub const BACKUP_CHUNK_BYTES: usize = 512 * 1024;

/// Octets d'un morceau d'envoi (`file.upload.chunk`), comme ceux des sauvegardes.
pub const UPLOAD_CHUNK_BYTES: usize = 512 * 1024;

/// Taille maximale d'un fichier envoyé depuis le panel.
pub const MAX_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;
