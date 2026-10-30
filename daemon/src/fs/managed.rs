//! Réécriture de clés dans un fichier existant (`managed-keys`) : seules les clés gérées
//! changent, le reste du fichier (commentaires, autres clés, ordre) est conservé.
//!
//! Les valeurs arrivent validées par l'API (aucun caractère de contrôle) ; elles sont tout de
//! même échappées pour le format.

use std::collections::BTreeMap;

use serde_json::Value;

/// Valeurs en texte, pour `properties` et `ini` : une chaîne telle quelle, un nombre ou un
/// booléen tels que JSON les écrit (l'API n'y envoie que des chaînes, la validation l'impose).
pub fn texts(keys: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    keys.iter()
        .map(|(key, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (key.clone(), text)
        })
        .collect()
}

/// JSON : chaque clé est un chemin `a.b.c` ; les objets manquants sont créés, le reste du
/// document est conservé, les types des valeurs aussi. Fichier vide ou absent : un objet.
/// Réécrit indenté (deux espaces) : la mise en forme d'origine n'est pas conservée.
pub fn json(original: &str, keys: &BTreeMap<String, Value>) -> Result<String, String> {
    let mut document: Value = if original.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(original).map_err(|e| format!("invalid json: {e}"))?
    };
    set_paths(&mut document, keys)?;
    let mut text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// YAML : mêmes règles qu'en JSON. Les commentaires et la mise en forme sont perdus à la
/// réécriture (le prefab le dit dans la description du fichier).
pub fn yaml(original: &str, keys: &BTreeMap<String, Value>) -> Result<String, String> {
    let mut document: Value = if original.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_saphyr::from_str(original).map_err(|e| format!("invalid yaml: {e}"))?
    };
    set_paths(&mut document, keys)?;
    serde_saphyr::to_string(&document).map_err(|e| e.to_string())
}

/// Pose chaque chemin dans le document, en créant les objets intermédiaires. Un segment qui
/// tombe sur une valeur qui n'est pas un objet est une erreur : Kuro n'écrase pas une structure
/// de l'utilisateur sans le dire.
fn set_paths(document: &mut Value, keys: &BTreeMap<String, Value>) -> Result<(), String> {
    if !document.is_object() {
        return Err("the document root is not an object".to_owned());
    }
    for (path, value) in keys {
        let mut node = &mut *document;
        let segments: Vec<&str> = path.split('.').collect();
        let (last, parents) = segments.split_last().ok_or("empty key")?;
        for segment in parents {
            let object = node
                .as_object_mut()
                .ok_or_else(|| format!("`{path}`: `{segment}` is not an object"))?;
            node = object
                .entry((*segment).to_owned())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if node.is_null() {
                *node = Value::Object(serde_json::Map::new());
            }
        }
        let object = node
            .as_object_mut()
            .ok_or_else(|| format!("`{path}`: parent of `{last}` is not an object"))?;
        object.insert((*last).to_owned(), value.clone());
    }
    Ok(())
}

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

#[cfg(test)]
mod structured_tests {
    use serde_json::json;

    use super::*;

    fn keys(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn json_paths_are_created_the_rest_is_kept_and_types_survive() {
        let original = "{\n  \"gateway\": { \"port\": 18789, \"trustedProxies\": [\"1.2.3.4\"] },\n  \"theme\": \"dark\"\n}\n";
        let written = json(
            original,
            &keys(&[
                (
                    "gateway.trustedProxies",
                    json!(["10.0.0.0/8", "172.16.0.0/12"]),
                ),
                (
                    "gateway.controlUi.allowedOrigins",
                    json!(["https://abc.web.kuroctl.test"]),
                ),
                ("gateway.port", json!(8080)),
                ("debug", json!(false)),
            ]),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(value["theme"], "dark", "untouched key kept");
        assert_eq!(value["gateway"]["port"], 8080, "number kept as a number");
        assert_eq!(value["debug"], false);
        assert_eq!(
            value["gateway"]["trustedProxies"],
            json!(["10.0.0.0/8", "172.16.0.0/12"])
        );
        assert_eq!(
            value["gateway"]["controlUi"]["allowedOrigins"],
            json!(["https://abc.web.kuroctl.test"]),
            "missing objects are created"
        );
        assert!(written.ends_with('\n'));
    }

    #[test]
    fn an_empty_or_missing_file_becomes_an_object_and_an_invalid_one_is_refused() {
        let written = json("", &keys(&[("a.b", json!("x"))])).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&written).unwrap(),
            json!({ "a": { "b": "x" } })
        );

        let error = json("{ not json", &keys(&[("a", json!(1))])).unwrap_err();
        assert!(error.starts_with("invalid json:"), "{error}");
        // Un segment qui n'est pas un objet n'est jamais écrasé en silence.
        let error = json("{\"a\": 1}", &keys(&[("a.b", json!(2))])).unwrap_err();
        assert!(error.contains("not an object"), "{error}");
        let error = json("[1, 2]", &keys(&[("a", json!(2))])).unwrap_err();
        assert!(error.contains("root"), "{error}");
    }

    #[test]
    fn yaml_follows_the_same_rules_and_loses_comments() {
        let original = "# commentaire\nmodel: gpt\nserver:\n  port: 9000\n";
        let written = yaml(
            original,
            &keys(&[
                ("server.public_url", json!("https://abc.web.kuroctl.test")),
                ("server.port", json!(8080)),
                ("server.trusted", json!(["10.0.0.0/8"])),
            ]),
        )
        .unwrap();
        let value: Value = serde_saphyr::from_str(&written).unwrap();
        assert_eq!(value["model"], "gpt");
        assert_eq!(value["server"]["port"], 8080);
        assert_eq!(
            value["server"]["public_url"],
            "https://abc.web.kuroctl.test"
        );
        assert_eq!(value["server"]["trusted"], json!(["10.0.0.0/8"]));
        assert!(!written.contains("commentaire"));

        let error = yaml("a: [", &keys(&[("a", json!(1))])).unwrap_err();
        assert!(error.starts_with("invalid yaml:"), "{error}");
    }

    #[test]
    fn texts_turn_scalars_into_strings_for_flat_formats() {
        let texts = texts(&keys(&[
            ("a", json!("x")),
            ("b", json!(5)),
            ("c", json!(true)),
        ]));
        assert_eq!(texts["a"], "x");
        assert_eq!(texts["b"], "5");
        assert_eq!(texts["c"], "true");
    }
}
