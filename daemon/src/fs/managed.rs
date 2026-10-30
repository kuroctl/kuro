//! Réécriture de clés dans un fichier existant (`managed-keys`) : seules les clés gérées
//! changent, le reste du fichier (commentaires, autres clés, ordre) est conservé.
//!
//! Les valeurs arrivent validées par l'API (aucun caractère de contrôle) ; elles sont tout de
//! même échappées pour le format.

use std::collections::BTreeMap;

/// `.properties` (Java) : `clé=valeur`, `clé: valeur` ou `clé valeur`.
pub fn properties(original: &str, keys: &BTreeMap<String, String>) -> String {
    let newline = newline_of(original);
    let mut pending: BTreeMap<&str, &str> =
        keys.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut lines: Vec<String> = original
        .lines()
        .map(|line| match property_key(line) {
            Some(key) => match pending.remove(key) {
                Some(value) => format!("{key}={}", escape_property(value)),
                None => line.to_owned(),
            },
            None => line.to_owned(),
        })
        .collect();
    for (key, value) in pending {
        lines.push(format!("{key}={}", escape_property(value)));
    }
    join(lines, newline)
}

fn property_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
        return None;
    }
    let end = trimmed
        .find(|c: char| c == '=' || c == ':' || c.is_whitespace())
        .unwrap_or(trimmed.len());
    Some(&trimmed[..end])
}

fn escape_property(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// `.ini` : clés `Section.Clé` (première partie avant le premier point), ou `Clé` hors section.
/// Sections et clés comparées sans tenir compte de la casse.
pub fn ini(original: &str, keys: &BTreeMap<String, String>) -> String {
    let newline = newline_of(original);
    // Section (minuscule) → clés à écrire, dans l'ordre.
    let mut pending: Vec<(String, String, String)> = keys
        .iter()
        .map(|(key, value)| match key.split_once('.') {
            Some((section, name)) => (section.to_owned(), name.to_owned(), value.clone()),
            None => (String::new(), key.clone(), value.clone()),
        })
        .collect();

    let mut lines: Vec<String> = Vec::new();
    let mut section = String::new();
    for line in original.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            flush_section(&mut lines, &mut pending, &section);
            section = name.to_owned();
            lines.push(line.to_owned());
            continue;
        }
        let key = (!trimmed.starts_with(';') && !trimmed.starts_with('#'))
            .then(|| trimmed.split_once('=').map(|(k, _)| k.trim()))
            .flatten();
        let replaced = key.and_then(|key| {
            let index = pending.iter().position(|(s, k, _)| {
                s.eq_ignore_ascii_case(&section) && k.eq_ignore_ascii_case(key)
            })?;
            let (_, _, value) = pending.remove(index);
            Some(format!("{key}={value}"))
        });
        lines.push(replaced.unwrap_or_else(|| line.to_owned()));
    }
    flush_section(&mut lines, &mut pending, &section);

    // Sections absentes du fichier : ajoutées à la fin.
    while let Some((name, _, _)) = pending.first().cloned() {
        if !lines.last().is_none_or(|l| l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{name}]"));
        flush_section(&mut lines, &mut pending, &name);
    }
    join(lines, newline)
}

/// Écrit les clés restantes de `section` à la fin de celle-ci (avant les lignes vides finales).
fn flush_section(
    lines: &mut Vec<String>,
    pending: &mut Vec<(String, String, String)>,
    section: &str,
) {
    let mut insert_at = lines.len();
    while insert_at > 0 && lines[insert_at - 1].trim().is_empty() {
        insert_at -= 1;
    }
    let mut index = 0;
    while index < pending.len() {
        if pending[index].0.eq_ignore_ascii_case(section) {
            let (_, key, value) = pending.remove(index);
            lines.insert(insert_at, format!("{key}={value}"));
            insert_at += 1;
        } else {
            index += 1;
        }
    }
}

fn newline_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

fn join(lines: Vec<String>, newline: &str) -> String {
    let mut out = lines.join(newline);
    out.push_str(newline);
    out
}
