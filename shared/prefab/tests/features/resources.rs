//! Ressources minimales et recommandées d'un prefab (ADR-0031).

use kuro_prefab::LoadError;
use kuro_prefab::manifest::ResourceNeed;

use crate::support::{Variant, official};

/// Terraria avec `resources` sous `requires`.
fn with_resources(resources: &str) -> Variant {
    Variant::of("terraria", |text| {
        text.replacen(
            "requires:\n  kuro: \">=0.2\"\n",
            &format!("requires:\n  kuro: \">=0.2\"\n  resources:\n{resources}"),
            1,
        )
    })
}

fn issues(variant: &Variant) -> Vec<String> {
    match variant.load() {
        Err(LoadError::Invalid(issues)) => issues.iter().map(ToString::to_string).collect(),
        other => panic!("expected validation issues, got {other:?}"),
    }
}

#[test]
fn declared_needs_are_read() {
    let variant = with_resources(
        "    memory_mb: { min: 1024, recommended: 2048 }\n    disk_mb: { recommended: 5120 }\n",
    );

    let resources = variant.load().unwrap().manifest.requires.resources;

    assert_eq!(
        resources.memory_mb,
        Some(ResourceNeed {
            min: Some(1024),
            recommended: Some(2048)
        })
    );
    assert_eq!(resources.min("memory_mb"), Some(1024));
    assert_eq!(resources.min("disk_mb"), None);
    assert_eq!(resources.min("cpu_percent"), None);
}

#[test]
fn a_minimum_above_the_recommendation_is_refused() {
    let variant = with_resources("    memory_mb: { min: 4096, recommended: 2048 }\n");

    assert!(
        issues(&variant)
            .iter()
            .any(|issue| issue.contains("requires.resources.memory_mb")
                && issue.contains("min must not exceed")),
        "{:?}",
        issues(&variant)
    );
}

#[test]
fn values_outside_what_a_server_accepts_are_refused() {
    let variant =
        with_resources("    memory_mb: { min: 32 }\n    cpu_percent: { recommended: 20000 }\n");

    let found = issues(&variant);
    assert!(
        found
            .iter()
            .any(|i| i.contains("requires.resources.memory_mb.min")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|i| i.contains("requires.resources.cpu_percent.recommended")),
        "{found:?}"
    );
}

#[test]
fn an_empty_need_and_unknown_keys_are_refused() {
    let empty = with_resources("    memory_mb: {}\n");
    let unknown = with_resources("    memory_mb: { max: 4096 }\n");

    assert!(
        issues(&empty)
            .iter()
            .any(|i| i.contains("requires.resources.memory_mb"))
    );
    assert!(matches!(unknown.load(), Err(LoadError::Manifest(_))));
}

#[test]
fn a_prefab_without_needs_serializes_as_before() {
    // Un manifest stocké avant ADR-0031 doit rester identique, sinon il passerait en conflit à
    // l'import suivant.
    let package = official("terraria");

    let json = serde_json::to_value(&package.manifest).unwrap();

    assert_eq!(json["requires"], serde_json::json!({ "kuro": ">=0.2" }));
}
