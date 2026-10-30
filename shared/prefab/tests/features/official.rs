//! Les prefabs livrés avec Kuro se chargent et passent toutes les validations.

use std::collections::HashSet;

use kuro_prefab::manifest::{FieldType, Section};

use crate::support::{official, official_dirs};

#[test]
fn every_official_prefab_loads() {
    let mut seen = HashSet::new();
    for dir in official_dirs() {
        let package = official(&dir);
        // Deux dossiers au même `id@version` entrent en conflit au rechargement du catalogue.
        let label = format!("{}@{}", package.manifest.id, package.manifest.version);
        assert!(seen.insert(label.clone()), "{dir}: {label} already used");
        assert!(package.locales.contains_key("en"));
        assert!(package.locales.contains_key("fr"));
    }
}

#[test]
fn settings_keep_the_declared_order() {
    let package = official("minecraft-java");

    let groups: Vec<(Section, &str)> = package
        .manifest
        .settings
        .fields()
        .map(|f| (f.section, f.group))
        .collect::<Vec<_>>()
        .into_iter()
        .fold(Vec::new(), |mut acc, item| {
            if acc.last() != Some(&item) {
                acc.push(item);
            }
            acc
        });

    assert_eq!(
        groups,
        vec![
            (Section::General, "server"),
            (Section::General, "version"),
            (Section::Advanced, "world"),
            (Section::Advanced, "security"),
        ]
    );
}

#[test]
fn templates_declared_by_the_manifest_are_loaded() {
    let package = official("ark-survival-ascended");

    assert!(package.templates.contains_key("templates/Game.ini"));
}

#[test]
fn generated_secrets_are_declared() {
    let package = official("wordpress");

    let secret = package.manifest.settings.field("db_password").unwrap();
    assert_eq!(secret.kind, FieldType::Secret);
    assert_eq!(
        kuro_prefab::settings::secrets_to_generate(&package.manifest),
        vec![("db_password".to_owned(), 32)]
    );
}
