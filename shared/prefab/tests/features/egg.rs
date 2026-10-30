//! Conversion d'un egg Pterodactyl (ADR-0027) : un prefab valide, le démarrage des images
//! Pterodactyl conservé, l'installation lancée sans root, chaque écart nommé.

use std::collections::BTreeMap;

use kuro_prefab::Package;
use kuro_prefab::compose::NoteKind;
use kuro_prefab::egg::{EggError, MAX_EGG_BYTES, convert};
use kuro_prefab::files::ArchiveFiles;
use kuro_prefab::manifest::{FileMode, Ready, Step, Stop};
use serde_json::json;

/// Egg de type Paper : images `yolks`, fichiers et démarrage encodés en chaînes comme dans un
/// export de Pterodactyl, script d'installation `ash` court.
fn paper() -> String {
    let text = json!({
        "_comment": "DO NOT EDIT: FILE GENERATED AUTOMATICALLY BY PTERODACTYL PANEL",
        "meta": { "version": "PTDL_v2", "update_url": null },
        "exported_at": "2026-01-01T00:00:00+00:00",
        "name": "Paper",
        "author": "someone@example.com",
        "description": "High performance Minecraft server.",
        "features": ["eula", "java_version", "pid_limit"],
        "docker_images": {
            "Java 21": "ghcr.io/pterodactyl/yolks:java_21",
            "Java 17": "ghcr.io/pterodactyl/yolks:java_17"
        },
        "file_denylist": [],
        "startup": "java -Xms128M -XX:MaxRAMPercentage=95.0 -jar {{SERVER_JARFILE}}",
        "config": {
            "files": "{\r\n    \"server.properties\": {\r\n        \"parser\": \"properties\",\r\n        \"find\": {\r\n            \"server-ip\": \"0.0.0.0\",\r\n            \"server-port\": \"{{server.build.default.port}}\",\r\n            \"query.port\": \"{{server.build.default.port}}\"\r\n        }\r\n    }\r\n}",
            "startup": "{\r\n    \"done\": \")! For help, type \"\r\n}",
            "logs": "{}",
            "stop": "stop"
        },
        "scripts": {
            "installation": {
                "script": "#!/bin/ash\r\ncd /mnt/server\r\ncurl -o ${SERVER_JARFILE} https://example.com/paper/${MINECRAFT_VERSION}/${BUILD_NUMBER}.jar\r\n",
                "container": "ghcr.io/pterodactyl/installers:alpine",
                "entrypoint": "ash"
            }
        },
        "variables": [
            { "name": "Minecraft Version", "description": "Version to install.", "env_variable": "MINECRAFT_VERSION",
              "default_value": "latest", "user_viewable": true, "user_editable": true,
              "rules": "nullable|string|max:20", "field_type": "text" },
            { "name": "Server Jar File", "description": "Jar file to run.", "env_variable": "SERVER_JARFILE",
              "default_value": "server.jar", "user_viewable": true, "user_editable": true,
              "rules": "required|regex:/^([\\w\\d._-]+)(\\.jar)$/", "field_type": "text" },
            { "name": "Build Number", "description": "", "env_variable": "BUILD_NUMBER",
              "default_value": "latest", "user_viewable": true, "user_editable": false,
              "rules": "required|string|max:20", "field_type": "text" }
        ]
    })
    .to_string();
    // `json!` trie les clés ; un export garde l'ordre de l'egg, la plus récente d'abord.
    let sorted = r#""docker_images":{"Java 17":"ghcr.io/pterodactyl/yolks:java_17","Java 21":"ghcr.io/pterodactyl/yolks:java_21"}"#;
    assert!(text.contains(sorted));
    text.replace(
        sorted,
        r#""docker_images":{"Java 21":"ghcr.io/pterodactyl/yolks:java_21","Java 17":"ghcr.io/pterodactyl/yolks:java_17"}"#,
    )
}

/// Egg de type SteamCMD : script `bash` qui installe des paquets, mot de passe, booléen, liste,
/// port déclaré par une variable, fichier JSON, arrêt par `^C`.
fn steam() -> String {
    json!({
        "meta": { "version": "PTDL_v2" },
        "name": "Viking Survival",
        "description": "Dedicated server.",
        "docker_images": { "Debian": "ghcr.io/parkervcp/games:source" },
        "startup": "./server -port {{SERVER_PORT}} -name \"{{SRV_NAME}}\" -password \"{{SRV_PASSWORD}}\" -public {{PUBLIC}}",
        "config": {
            "files": { "config/settings.json": { "parser": "json", "find": { "server.port": "{{server.build.default.port}}" } } },
            "startup": { "done": ["Game server connected", "Server ready"] },
            "logs": {},
            "stop": "^C"
        },
        "scripts": {
            "installation": {
                "script": "#!/bin/bash\napt update\napt install -y curl lib32gcc-s1\nmkdir -p /mnt/server/steamcmd\n",
                "container": "ghcr.io/parkervcp/installers:debian",
                "entrypoint": "bash"
            }
        },
        "variables": [
            { "name": "Server Name", "env_variable": "SRV_NAME", "default_value": "My server",
              "user_viewable": true, "user_editable": true, "rules": "required|string|max:64" },
            { "name": "Password", "env_variable": "SRV_PASSWORD", "default_value": "",
              "user_viewable": true, "user_editable": true, "rules": "nullable|string|min:5" },
            { "name": "Public", "env_variable": "PUBLIC", "default_value": "1",
              "user_viewable": true, "user_editable": true, "rules": "required|boolean" },
            { "name": "World", "env_variable": "WORLD_TYPE", "default_value": "normal",
              "user_viewable": true, "user_editable": true, "rules": "required|in:normal,hard,hardcore" },
            { "name": "Game Port", "env_variable": "GAME_PORT", "default_value": "2456",
              "user_viewable": false, "user_editable": false, "rules": "required|integer|between:1024,65535" }
        ]
    })
    .to_string()
}

fn package(files: &BTreeMap<String, String>) -> Package {
    let archive = ArchiveFiles::from_map(
        files
            .iter()
            .map(|(path, text)| (path.clone(), text.as_bytes().to_vec()))
            .collect(),
    )
    .unwrap();
    Package::load(&archive).unwrap_or_else(|error| {
        panic!(
            "converted prefab is invalid: {error}\n{}",
            files["prefab.yaml"]
        )
    })
}

fn has_note(conversion: &kuro_prefab::compose::Conversion, path: &str, kind: NoteKind) -> bool {
    conversion
        .notes
        .iter()
        .any(|note| note.path == path && note.kind == kind)
}

#[test]
fn a_paper_egg_becomes_a_valid_prefab_that_keeps_the_image_startup() {
    let conversion = convert(&paper(), None).unwrap();

    assert_eq!(conversion.id, "paper");
    let package = package(&conversion.files);
    let server = &package.manifest.services["server"];
    assert_eq!(server.image, "ghcr.io/pterodactyl/yolks:java_21");
    // L'image lit `STARTUP` ; la valeur du réglage arrive par l'environnement, jamais collée.
    assert_eq!(
        server.env["STARTUP"],
        "java -Xms128M -XX:MaxRAMPercentage=95.0 -jar ${SERVER_JARFILE}"
    );
    assert_eq!(
        server.env["SERVER_JARFILE"],
        "{{ settings.server_jarfile }}"
    );
    assert_eq!(server.env["SERVER_PORT"], "{{ ports.game.container }}");
    assert_eq!(server.env["SERVER_MEMORY"], "{{ resources.memory_mb }}");
    assert!(server.command.is_empty(), "the image entrypoint is kept");
    assert_eq!(server.ports["game"].container, 25565);
    assert!(server.ports["game"].public);
    assert_eq!(server.volumes["data"].path, "/home/container");

    let properties = &package.manifest.files["server-properties"];
    assert_eq!(properties.path, "/home/container/server.properties");
    assert_eq!(properties.mode, FileMode::ManagedKeys);
    assert_eq!(properties.keys["server-port"], "{{ ports.game.container }}");
    assert_eq!(properties.keys["server-ip"], "0.0.0.0");

    assert_eq!(
        package.manifest.lifecycle.ready,
        Some(Ready::Console(r"\)! For help, type ".to_owned()))
    );
    assert_eq!(
        package.manifest.lifecycle.stop,
        Stop::Console("stop".to_owned())
    );

    let install = package.manifest.install.as_ref().unwrap();
    let Step::Script { image, run } = &install.steps[0] else {
        panic!("expected a script step");
    };
    assert_eq!(
        image.as_deref(),
        Some("ghcr.io/pterodactyl/installers:alpine")
    );
    assert!(run.contains("cd /home/container"), "{run}");
    assert!(!run.contains("/mnt/server"));
    assert!(run.contains("export MINECRAFT_VERSION='latest'"), "{run}");
    assert!(!run.contains('\r'));

    assert!(has_note(&conversion, "docker_images", NoteKind::Adapted));
    assert!(has_note(
        &conversion,
        "docker_images.Java 17",
        NoteKind::Ignored
    ));
    assert!(has_note(&conversion, "author", NoteKind::Ignored));
    assert!(
        !conversion.files["prefab.yaml"].contains("someone@example.com"),
        "the author e-mail is personal data"
    );
    let en: serde_json::Value = serde_json::from_str(&conversion.files["locales/en.json"]).unwrap();
    assert_eq!(en["prefab"]["name"], "Paper");
    assert_eq!(en["fields"]["server_jarfile"]["label"], "Server Jar File");
    assert_eq!(en["fields"]["server_jarfile"]["help"], "Jar file to run.");
}

#[test]
fn laravel_rules_become_typed_settings() {
    let conversion = convert(&steam(), None).unwrap();
    let package = package(&conversion.files);
    let manifest = &conversion.files["prefab.yaml"];

    assert!(manifest.contains("srv_name: { type: string, default: \"My server\", max_length: 64, visibility: customer, restart: true }"), "{manifest}");
    assert!(
        manifest.contains("srv_password: { type: password"),
        "{manifest}"
    );
    assert!(
        manifest.contains("public: { type: enum, default: \"1\", options: [\"0\", \"1\"]"),
        "{manifest}"
    );
    assert!(manifest.contains("world_type: { type: enum, default: \"normal\", options: [\"normal\", \"hard\", \"hardcore\"]"), "{manifest}");
    // Non visible pour l'utilisateur : réservé à l'admin.
    assert!(
        manifest.contains(
            "game_port: { type: int, default: 2456, min: 1024, max: 65535, restart: true }"
        ),
        "{manifest}"
    );
    assert_eq!(
        package.manifest.services["server"].ports["game"].container,
        2456
    );
    assert_eq!(
        package.manifest.services["server"].env["STARTUP"],
        "./server -port ${SERVER_PORT} -name \"${SRV_NAME}\" -password \"${SRV_PASSWORD}\" -public ${PUBLIC}"
    );
}

#[test]
fn an_install_that_needs_root_is_kept_but_refused() {
    let conversion = convert(&steam(), None).unwrap();
    let package = package(&conversion.files);

    assert!(has_note(
        &conversion,
        "scripts.installation.script",
        NoteKind::Refused
    ));
    let manifest = &conversion.files["prefab.yaml"];
    assert!(manifest.starts_with("# Converti depuis un egg"));
    assert!(manifest.contains("# refusé : scripts.installation.script"));
    let Step::Script { run, .. } = &package.manifest.install.as_ref().unwrap().steps[0] else {
        panic!("expected a script step");
    };
    // Script bash : passé à bash, le prefab lançant `sh -c`.
    assert!(run.starts_with("exec bash <<'KURO_EGG_INSTALL'\n"), "{run}");
    assert!(run.contains("mkdir -p /home/container/steamcmd"));
    assert_eq!(
        package.manifest.lifecycle.stop,
        Stop::Signal(kuro_prefab::manifest::Signal::Int)
    );
    assert_eq!(
        package.manifest.lifecycle.ready,
        Some(Ready::Console(
            "(Game server connected|Server ready)".to_owned()
        ))
    );
}

#[test]
fn files_kuro_cannot_rewrite_stay_editable_by_hand() {
    let conversion = convert(&steam(), None).unwrap();
    let package = package(&conversion.files);

    let settings = &package.manifest.files["config-settings-json"];
    assert_eq!(settings.mode, FileMode::User);
    assert_eq!(settings.path, "/home/container/config/settings.json");
    assert!(has_note(
        &conversion,
        "config.files.config/settings.json.find",
        NoteKind::Refused
    ));
}

#[test]
fn default_values_cannot_inject_shell_in_the_install_script() {
    let mut egg: serde_json::Value = serde_json::from_str(&paper()).unwrap();
    egg["variables"][0]["default_value"] = json!("1.21'; rm -rf / #");

    let conversion = convert(&egg.to_string(), None).unwrap();
    let package = package(&conversion.files);

    let Step::Script { run, .. } = &package.manifest.install.as_ref().unwrap().steps[0] else {
        panic!("expected a script step");
    };
    assert!(
        run.contains(r"export MINECRAFT_VERSION='1.21'\''; rm -rf / #'"),
        "{run}"
    );
}

#[test]
fn an_unknown_reference_in_a_file_is_refused() {
    let mut egg: serde_json::Value = serde_json::from_str(&paper()).unwrap();
    egg["config"]["files"] = json!({
        "server.properties": { "parser": "properties", "find": {
            "server-port": "{{server.build.default.port}}",
            "server-ip": "{{config.docker.interface}}"
        } }
    });

    let conversion = convert(&egg.to_string(), None).unwrap();
    let package = package(&conversion.files);

    let keys = &package.manifest.files["server-properties"].keys;
    assert!(keys.contains_key("server-port"));
    assert!(!keys.contains_key("server-ip"));
    assert!(has_note(
        &conversion,
        "config.files.server.properties.find.server-ip",
        NoteKind::Refused
    ));
}

#[test]
fn what_is_not_an_egg_is_rejected() {
    assert!(matches!(
        convert("not json", None),
        Err(EggError::Unreadable(_))
    ));
    assert!(matches!(
        convert(r#"{"meta": {"version": "other"}, "name": "x"}"#, None),
        Err(EggError::NotAnEgg(_))
    ));
    assert!(matches!(
        convert(
            r#"{"meta": {"version": "PTDL_v2"}, "name": "x", "startup": "run"}"#,
            None
        ),
        Err(EggError::NotAnEgg(_))
    ));
    let huge = format!("{{\"pad\": \"{}\"}}", "x".repeat(MAX_EGG_BYTES));
    assert!(matches!(convert(&huge, None), Err(EggError::TooLarge)));
}

#[test]
fn the_wanted_id_is_used() {
    let conversion = convert(&paper(), Some("My Paper")).unwrap();

    assert_eq!(conversion.id, "my-paper");
    assert!(conversion.files["prefab.yaml"].contains("id: my-paper\n"));
}

/// Egg FiveM réduit : port de txAdmin en variable, licence, paquets installés par le script.
fn fivem() -> String {
    json!({
        "meta": { "version": "PTDL_v2" },
        "name": "FiveM",
        "docker_images": { "Debian": "ghcr.io/ptero-eggs/yolks:debian" },
        "startup": "./FXServer +set sv_licenseKey {{FIVEM_LICENSE}}",
        "config": { "files": "{}", "startup": "{\"done\": \"Welcome!\"}", "logs": "{}", "stop": "quit" },
        "scripts": { "installation": {
            "script": "#!/bin/bash\napt update -y\napt install -y tar xz-utils file jq\nmkdir -p /mnt/server/resources\n",
            "container": "ghcr.io/ptero-eggs/installers:debian", "entrypoint": "bash" } },
        "variables": [
            { "name": "License", "env_variable": "FIVEM_LICENSE", "default_value": "",
              "user_viewable": true, "user_editable": true, "rules": "required|string|max:33" },
            { "name": "txAdmin Port", "env_variable": "TXHOST_TXA_PORT", "default_value": "40120",
              "user_viewable": true, "user_editable": false, "rules": "required|string|max:20" }
        ]
    })
    .to_string()
}

#[test]
fn a_side_port_is_not_taken_for_the_game_port() {
    let conversion = convert(&fivem(), None).unwrap();
    let package = package(&conversion.files);
    let ports = &package.manifest.services["server"].ports;

    // txAdmin garde son port ; celui du jeu est inconnu de l'egg, et signalé comme tel.
    assert_eq!(ports["txhost_txa_port"].container, 40120);
    assert_ne!(ports["game"].container, 40120);
    assert!(has_note(&conversion, "startup", NoteKind::Adapted));
    assert!(has_note(
        &conversion,
        "variables.TXHOST_TXA_PORT",
        NoteKind::Adapted
    ));
}

#[test]
fn a_licence_is_a_secret_and_missing_packages_are_named() {
    let conversion = convert(&fivem(), None).unwrap();
    let manifest = &conversion.files["prefab.yaml"];

    assert!(
        manifest.contains("fivem_license: { type: password"),
        "{manifest}"
    );
    let refused = conversion
        .notes
        .iter()
        .find(|note| note.path == "scripts.installation.script" && note.kind == NoteKind::Refused)
        .unwrap();
    assert!(
        refused.message.contains("(tar, xz-utils, file, jq)"),
        "{}",
        refused.message
    );
}
