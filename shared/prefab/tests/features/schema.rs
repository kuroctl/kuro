//! Le JSON Schema publié et la validation de Kuro disent la même chose : mêmes prefabs acceptés,
//! mêmes pièges refusés. Certains contrôles croisés n'existent que côté Kuro.

use kuro_prefab::LoadError;
use serde_json::Value;

use crate::support::{OFFICIAL, Variant, manifest_json, official, prefabs_dir};

fn schema() -> jsonschema::Validator {
    let text = std::fs::read_to_string(prefabs_dir().join("schema/prefab.v1.schema.json")).unwrap();
    let schema: Value = serde_json::from_str(&text).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

#[test]
fn official_prefabs_match_the_published_schema() {
    let schema = schema();
    for id in OFFICIAL {
        let instance = manifest_json(&prefabs_dir().join(id));
        let errors: Vec<String> = schema
            .iter_errors(&instance)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{id}: {errors:?}");
    }
}

/// Cas piégés : refusés par Kuro, et par le schéma quand il peut l'exprimer.
struct Trap {
    name: &'static str,
    from: &'static str,
    to: &'static str,
    schema_rejects: bool,
}

const TRAPS: &[Trap] = &[
    Trap {
        name: "privileged container",
        from: "    user: \"1000:1000\"\n",
        to: "    user: \"1000:1000\"\n    privileged: true\n",
        schema_rejects: true,
    },
    Trap {
        name: "host network",
        from: "    user: \"1000:1000\"\n",
        to: "    user: \"1000:1000\"\n    network_mode: host\n",
        schema_rejects: true,
    },
    Trap {
        name: "root user",
        from: "user: \"1000:1000\"",
        to: "user: \"0:0\"",
        schema_rejects: true,
    },
    Trap {
        name: "latest tag",
        from: "minecraft-server:2026.10.1-java25",
        to: "minecraft-server:latest",
        schema_rejects: true,
    },
    Trap {
        name: "image without tag",
        from: "minecraft-server:2026.10.1-java25",
        to: "minecraft-server",
        schema_rejects: true,
    },
    Trap {
        name: "path traversal",
        from: "install: { mode: download, dir: /data/mods }",
        to: "install: { mode: download, dir: /data/../etc }",
        schema_rejects: true,
    },
    Trap {
        name: "unknown root key",
        from: "category: game\n",
        to: "category: game\nhooks: [rm -rf /]\n",
        schema_rejects: true,
    },
    Trap {
        name: "file outside every volume",
        from: "path: /data/ops.json",
        to: "path: /etc/ops.json",
        schema_rejects: false,
    },
    Trap {
        name: "unknown setting in a template",
        from: "\"{{ settings.motd }}\"",
        to: "\"{{ settings.mtod }}\"",
        schema_rejects: false,
    },
    Trap {
        name: "default out of bounds",
        from: "max_players: { type: int, default: 20,",
        to: "max_players: { type: int, default: 9000,",
        schema_rejects: false,
    },
    Trap {
        name: "templating a non-public port host",
        from: "rcon.port: \"{{ ports.rcon.container }}\"",
        to: "rcon.port: \"{{ ports.rcon.host }}\"",
        schema_rejects: false,
    },
];

#[test]
fn traps_are_rejected() {
    let schema = schema();
    for trap in TRAPS {
        let variant = Variant::of("minecraft-java", |text| {
            text.replacen(trap.from, trap.to, 1)
        });

        assert!(
            matches!(
                variant.load(),
                Err(LoadError::Invalid(_) | LoadError::Manifest(_))
            ),
            "{}: accepted by Kuro",
            trap.name
        );
        assert_eq!(
            !schema.is_valid(&variant.manifest_json()),
            trap.schema_rejects,
            "{}: schema disagrees",
            trap.name
        );
    }
}

#[test]
fn extensions_are_accepted() {
    let variant = Variant::of("minecraft-java", |text| {
        text.replacen(
            "category: game\n",
            "category: game\nx-studio: { draft: true }\n",
            1,
        )
    });

    assert!(variant.load().is_ok());
    assert!(schema().is_valid(&variant.manifest_json()));
}

#[test]
fn missing_translation_is_reported() {
    let variant = Variant::copy("minecraft-java");
    let fr = variant.path().join("locales/fr.json");
    let mut messages: Value = serde_json::from_str(&std::fs::read_to_string(&fr).unwrap()).unwrap();
    messages["fields"].as_object_mut().unwrap().remove("pvp");
    std::fs::write(&fr, messages.to_string()).unwrap();

    let Err(LoadError::Invalid(issues)) = variant.load() else {
        panic!("missing translation accepted");
    };
    assert!(
        issues
            .iter()
            .any(|i| i.path == "locales/fr.json" && i.message.contains("fields.pvp.label"))
    );
}

#[test]
fn settings_migrations_are_checked() {
    let load = |migrations: &str| {
        Variant::of("minecraft-java", |text| {
            text.replacen(
                "lifecycle:\n",
                &format!("migrations:\n{migrations}lifecycle:\n"),
                1,
            )
        })
        .load()
    };
    let issues = |migrations: &str| match load(migrations) {
        Err(LoadError::Invalid(issues)) => issues
            .iter()
            .map(|issue| issue.path.clone())
            .collect::<Vec<_>>(),
        other => panic!("expected invalid, got {other:?}"),
    };

    assert!(
        load("  - { from: \"<1.1.0\", rename: { message: motd }, remove: [legacy] }\n").is_ok()
    );
    assert_eq!(
        issues("  - { from: \"not a range\" }\n"),
        ["migrations.0.from"]
    );
    assert_eq!(
        issues("  - { from: \"*\", rename: { message: nope } }\n"),
        ["migrations.0.rename.message"]
    );
    assert_eq!(
        issues("  - { from: \"*\", rename: { message: motd }, remove: [message] }\n"),
        ["migrations.0.rename.message"]
    );
}

#[test]
fn content_fields_are_checked() {
    let issues = |variant: Variant| match variant.load() {
        Err(LoadError::Invalid(issues)) => issues
            .iter()
            .map(|issue| format!("{}: {}", issue.path, issue.message))
            .collect::<Vec<_>>(),
        other => panic!("expected invalid, got {other:?}"),
    };

    // Un champ hors du mode `reference` : ses valeurs n'atteindraient jamais les gabarits.
    let download = issues(Variant::of("minecraft-java", |text| {
        text.replacen(
            "    install: { mode: download, dir: /data/mods }\n",
            "    install: { mode: download, dir: /data/mods }\n    fields:\n      note: { type: string }\n",
            1,
        )
    }));
    assert!(
        download
            .iter()
            .any(|i| i.contains("fields need install mode reference")),
        "{download:?}"
    );

    // Un gabarit qui cite un champ non déclaré.
    let unknown = issues(Variant::of("project-zomboid", |text| {
        text.replacen("values.mod_ids", "values.mods_ids", 1)
    }));
    assert!(
        unknown
            .iter()
            .any(|i| i.contains("unknown content field `mods.mods_ids`")),
        "{unknown:?}"
    );

    // Le libellé du champ est exigé dans chaque langue.
    let variant = Variant::copy("project-zomboid");
    let fr = variant.path().join("locales/fr.json");
    let mut messages: Value = serde_json::from_str(&std::fs::read_to_string(&fr).unwrap()).unwrap();
    messages["content"]["mods"]
        .as_object_mut()
        .unwrap()
        .remove("fields");
    std::fs::write(&fr, messages.to_string()).unwrap();
    let missing = issues(variant);
    assert!(
        missing
            .iter()
            .any(|i| i.contains("content.mods.fields.mod_ids.label")),
        "{missing:?}"
    );
}

#[test]
fn a_prefill_pattern_finds_the_values_in_a_description() {
    let package = official("project-zomboid");
    let prefill = package.manifest.content["mods"].fields["mod_ids"]
        .prefill
        .clone()
        .unwrap();
    let description = "Workshop ID: 2392709985\nMod ID: Hydrocraft\n  mod ids : HCFood; HCTools, HCFood\nSee also: Mod ID in the readme";

    assert_eq!(
        prefill.values(description),
        ["Hydrocraft", "HCFood", "HCTools"]
    );
    assert!(prefill.values("No identifier here.").is_empty());
}

#[test]
fn a_prefill_pattern_must_compile_and_capture() {
    let issues = |pattern: &str| {
        let variant = Variant::of("project-zomboid", |text| {
            let start = text.find("description: '").unwrap() + "description: '".len();
            let end = start + text[start..].find("' }").unwrap();
            format!("{}{pattern}{}", &text[..start], &text[end..])
        });
        match variant.load() {
            Err(LoadError::Invalid(issues)) => issues
                .iter()
                .map(|issue| format!("{}: {}", issue.path, issue.message))
                .collect::<Vec<_>>(),
            other => panic!("expected invalid, got {other:?}"),
        }
    };

    assert!(
        issues("Mod ID: .+")
            .iter()
            .any(|i| i.contains("the pattern needs a capture group"))
    );
    assert!(
        issues("Mod ID: (")
            .iter()
            .any(|i| i.contains("invalid regular expression"))
    );
}

#[test]
fn an_extracted_slot_routes_under_a_volume_and_registers_item_values() {
    let issues = |edit: fn(String) -> String| match Variant::of("minecraft-bedrock", edit).load() {
        Err(LoadError::Invalid(issues)) => issues
            .iter()
            .map(|issue| format!("{}: {}", issue.path, issue.message))
            .collect::<Vec<_>>(),
        other => panic!("expected invalid, got {other:?}"),
    };

    let entry = issues(|text| {
        text.replacen(
            "pack_id: \"{{ item.header.uuid }}\"",
            "pack_id: \"pack-{{ item.header.uuid }}\"",
            1,
        )
    });
    assert!(
        entry
            .iter()
            .any(|i| i.contains("register[0].entry.pack_id")),
        "{entry:?}"
    );
    let route = issues(|text| text.replacen("dir: /data/behavior_packs", "dir: /etc/packs", 1));
    assert!(
        route.iter().any(|i| i.contains("install.route[0].dir")),
        "{route:?}"
    );
    let describe =
        issues(|text| text.replacen("describe: manifest.json", "describe: ../manifest.json", 1));
    assert!(
        describe.iter().any(|i| i.contains("install.describe")),
        "{describe:?}"
    );
}
