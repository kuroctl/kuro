//! Motifs `ignore` des sauvegardes, en chemins de volumes (`server/data/logs/**`) : `*` couvre
//! une partie d'un segment, `**` un nombre quelconque de segments (zéro compris).

#[derive(Debug, Clone, Default)]
pub struct Ignore {
    patterns: Vec<Vec<String>>,
}

impl Ignore {
    /// `None` si un motif est vide ou contient `**` ailleurs qu'en segment entier.
    pub fn new(patterns: &[String]) -> Option<Self> {
        let mut parsed = Vec::with_capacity(patterns.len());
        for pattern in patterns {
            let segments: Vec<String> = pattern
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if segments.is_empty() || segments.iter().any(|s| s.contains("**") && s != "**") {
                return None;
            }
            parsed.push(segments);
        }
        Some(Self { patterns: parsed })
    }

    /// Le chemin (`[service, volume, …]`) est exclu, et avec lui tout ce qu'il contient.
    pub fn matches(&self, path: &[&str]) -> bool {
        self.patterns.iter().any(|pattern| glob(pattern, path))
    }
}

fn glob(pattern: &[String], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((first, rest)) if first == "**" => {
            (0..=path.len()).any(|skip| glob(rest, &path[skip..]))
        }
        Some((first, rest)) => match path.split_first() {
            Some((segment, others)) => segment_matches(first, segment) && glob(rest, others),
            None => false,
        },
    }
}

/// `*` couvre n'importe quelle suite de caractères dans un segment.
fn segment_matches(pattern: &str, segment: &str) -> bool {
    let mut parts = pattern.split('*');
    let Some(first) = parts.next() else {
        return pattern == segment;
    };
    let Some(mut rest) = segment.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // Aucun joker.
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}
