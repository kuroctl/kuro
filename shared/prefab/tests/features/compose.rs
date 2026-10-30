//! Import d'un `docker-compose.yml` : ce qui est sûr est converti en un prefab valide, le reste
//! est refusé et nommé ; les variables deviennent des réglages.

use kuro_prefab::Package;
use kuro_prefab::compose::{ComposeError, MAX_COMPOSE_BYTES, NoteKind, convert};
use kuro_prefab::files::ArchiveFiles;

const WORDPRESS: &str = r#"
name: My Blog
services:
  wordpress:
    image: wordpress:6.8-php8.3-apache
    restart: always
    ports:
      - "8080:80"
    environment:
      WORDPRESS_DB_HOST: db
      WORDPRESS_DB_PASSWORD: ${DB_PASSWORD}
      WORDPRESS_TABLE_PREFIX: ${PREFIX:-wp_}
    volumes:
      - html:/var/www/html
    user: "33:33"
    depends_on:
      db:
        condition: service_healthy
  db:
    image: mariadb:11.4
    environment:
      - MARIADB_PASSWORD=${DB_PASSWORD}
    volumes:
      - db_data:/var/lib/mysql
    tmpfs: /run/mysqld
    user: "999:999"
    healthcheck:
      test: ["CMD", "healthcheck.sh", "--connect"]
      interval: 10s
      retries: 5
volumes:
  html:
  db_data:
"#;

fn package(files: &std::collections::BTreeMap<String, String>) -> Package {
    let files = ArchiveFiles::from_map(
        files
            .iter()
            .map(|(path, text)| (path.clone(), text.as_bytes().to_vec()))
            .collect(),
    )
    .unwrap();
    Package::load(&files)
        .unwrap_or_else(|error| panic!("{error}\n{}", "converted prefab is invalid"))
}

#[test]
fn a_compose_with_safe_keys_becomes_a_valid_prefab() {
    let conversion = convert(WORDPRESS, None).unwrap();

    assert_eq!(conversion.id, "my-blog");
    let manifest = &conversion.files["prefab.yaml"];
    let package = package(&conversion.files);
    let web = &package.manifest.services["wordpress"];
    assert_eq!(web.image, "docker.io/library/wordpress:6.8-php8.3-apache");
    assert_eq!(web.ports["p80"].container, 80);
    assert!(web.ports["p80"].public);
    assert_eq!(web.volumes["html"].path, "/var/www/html");
    assert_eq!(web.depends_on, vec!["db"]);
    assert_eq!(
        web.env["WORDPRESS_DB_PASSWORD"],
        "{{ settings.db_password }}"
    );
    let db = &package.manifest.services["db"];
    assert!(db.writable.contains(&"/run/mysqld".to_owned()));
    assert_eq!(
        db.healthcheck.as_ref().unwrap().command,
        vec!["healthcheck.sh", "--connect"]
    );
    assert_eq!(package.manifest.lifecycle.service, "wordpress");
    assert!(manifest.contains("category: web"), "{manifest}");
    // Le mot de passe devient un secret généré, la variable avec défaut un réglage.
    assert!(
        manifest.contains("db_password: { type: secret"),
        "{manifest}"
    );
    assert!(
        manifest.contains(r#"prefix: { type: string, default: "wp_""#),
        "{manifest}"
    );
    assert!(
        conversion
            .notes
            .iter()
            .any(|note| note.path == "services.wordpress.restart" && note.kind == NoteKind::Ignored)
    );
    assert!(
        conversion
            .notes
            .iter()
            .any(|note| note.path == "services.wordpress.ports[0]"
                && note.kind == NoteKind::Adapted
                && note.message.contains("attribué par Kuro"))
    );
}

#[test]
fn the_first_service_of_the_file_names_the_prefab() {
    let compose = "services:
  wordpress:
    image: docker.io/library/wordpress:6.8
  db:
    image: docker.io/library/mariadb:11.4
";
    let conversion = convert(compose, None).unwrap();

    assert_eq!(conversion.id, "wordpress");
    let manifest = &conversion.files["prefab.yaml"];
    assert!(
        manifest.find("  wordpress:").unwrap() < manifest.find("  db:").unwrap(),
        "{manifest}"
    );
}

#[test]
fn whatever_opens_the_host_is_refused_by_name() {
    let compose = r#"
services:
  agent:
    image: ghcr.io/example/agent:1.2.3
    privileged: true
    network_mode: host
    cap_add: [SYS_ADMIN]
    devices: ["/dev/kmsg:/dev/kmsg"]
    user: root
    volumes:
      - /:/host
      - /var/run/docker.sock:/var/run/docker.sock
  builder:
    build: .
"#;
    let conversion = convert(compose, Some("agent")).unwrap();

    let refused: Vec<&str> = conversion
        .notes
        .iter()
        .filter(|note| note.kind == NoteKind::Refused)
        .map(|note| note.path.as_str())
        .collect();
    for path in [
        "services.agent.privileged",
        "services.agent.network_mode",
        "services.agent.cap_add",
        "services.agent.devices",
        "services.agent.user",
        "services.builder.build",
        "services.builder",
    ] {
        assert!(refused.contains(&path), "{path} not refused: {refused:?}");
    }
    let manifest = &conversion.files["prefab.yaml"];
    for word in [
        "privileged:",
        "network_mode",
        "cap_add",
        "devices",
        "builder:",
    ] {
        assert!(
            !manifest
                .lines()
                .any(|line| !line.starts_with('#') && line.contains(word)),
            "{word} in\n{manifest}"
        );
    }
    // Les montages de l'hôte deviennent des volumes du serveur.
    let package = package(&conversion.files);
    let agent = &package.manifest.services["agent"];
    assert_eq!(agent.volumes.len(), 2);
    assert!(agent.volumes.values().any(|volume| volume.path == "/host"));
    assert!(agent.user.is_none());
}

#[test]
fn commands_are_split_and_a_double_dollar_stays_literal() {
    let compose = r#"
services:
  app:
    image: docker.io/library/alpine:3.20
    entrypoint: /bin/sh
    command: -c "echo $$HOME && sleep ${DELAY:-5}"
"#;
    let conversion = convert(compose, None).unwrap();

    let package = package(&conversion.files);
    assert_eq!(
        package.manifest.services["app"].command,
        vec!["-c", "echo $HOME && sleep {{ settings.delay }}"]
    );
    // Le point d'entrée du compose n'est pas repris : l'image garde le sien, et la note le dit
    // (ADR-0039) plutôt que de fusionner en silence.
    assert!(
        conversion
            .notes
            .iter()
            .any(|note| note.path == "services.app.entrypoint"
                && note.kind == NoteKind::Refused
                && note.message.contains("/bin/sh")),
        "{:?}",
        conversion.notes
    );
}

#[test]
fn an_unpinned_image_is_flagged_and_left_for_the_validation() {
    let conversion = convert(
        "services:\n  web:\n    image: nginx\n    ports: [\"80\"]\n",
        None,
    )
    .unwrap();

    assert!(
        conversion
            .notes
            .iter()
            .any(|note| note.path == "services.web.image" && note.message.contains("version datée"))
    );
    assert!(conversion.files["prefab.yaml"].contains("image: \"docker.io/library/nginx\""));
}

#[test]
fn something_that_is_not_a_compose_file_is_refused() {
    assert!(matches!(
        convert("just: text\n", None),
        Err(ComposeError::NoService)
    ));
    assert!(matches!(
        convert("services: [", None),
        Err(ComposeError::Unreadable(_))
    ));
    let huge = format!(
        "services:\n  a:\n    image: x\n#{}",
        "x".repeat(MAX_COMPOSE_BYTES)
    );
    assert!(matches!(convert(&huge, None), Err(ComposeError::TooLarge)));
}
