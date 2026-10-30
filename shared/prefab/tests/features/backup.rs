//! Section `backup` (ADR-0018) : validation et traduction en chemins de volumes.

use kuro_prefab::{LoadError, resolve_backup};
use kuro_protocol::api::BackupHook;

use crate::support::{Variant, official};

#[test]
fn minecraft_saves_its_world_around_the_archive() {
    let plan = resolve_backup(&official("minecraft-java"));

    assert_eq!(plan.ignore, ["server/data/logs/**"]);
    assert_eq!(
        plan.before,
        [
            BackupHook {
                console: "save-off".to_owned(),
                wait: None,
            },
            BackupHook {
                console: "save-all flush".to_owned(),
                wait: Some("Saved the game".to_owned()),
            },
        ]
    );
    assert_eq!(plan.after.len(), 1);
}

#[test]
fn a_prefab_without_backup_section_archives_everything() {
    let plan = resolve_backup(&official("wordpress"));

    assert!(plan.ignore.is_empty() && plan.before.is_empty() && plan.after.is_empty());
}

#[test]
fn invalid_backup_sections_are_refused() {
    for (edit, path, message) in [
        (
            "server: [/etc/**]",
            "backup.ignore.server[0]",
            "not under a volume",
        ),
        (
            "nope: [/data/x]",
            "backup.ignore.nope[0]",
            "unknown service",
        ),
        (
            "server: [/data/a**b]",
            "backup.ignore.server[0]",
            "whole path segment",
        ),
    ] {
        let variant = Variant::of("minecraft-java", |text| {
            text.replacen("server: [/data/logs/**]", edit, 1)
        });
        let Err(LoadError::Invalid(issues)) = variant.load() else {
            panic!("`{edit}` accepted");
        };
        assert!(
            issues
                .iter()
                .any(|i| i.path == path && i.message.contains(message)),
            "{edit}: {issues:?}"
        );
    }

    let variant = Variant::of("minecraft-java", |text| {
        text.replacen("wait: 'Saved the game'", "wait: '(unclosed'", 1)
    });
    let Err(LoadError::Invalid(issues)) = variant.load() else {
        panic!("invalid regex accepted");
    };
    assert!(issues.iter().any(|i| i.path == "backup.before[1]"));
}
