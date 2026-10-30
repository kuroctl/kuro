//! Le daemon applique ses propres règles à toute spécification reçue (ADR-0013).

use kuro_daemon::server::spec::check;
use kuro_protocol::api::server::{PortProtocol, PortSpec, PublishedPort, ReadySpec};
use uuid::Uuid;

use super::support::{service, single};

#[test]
fn a_valid_spec_is_accepted() {
    assert_eq!(check(&single(Uuid::now_v7())), Ok(()));
}

/// Modification d'une spécification valide.
type Edit = Box<dyn Fn(&mut kuro_protocol::api::server::ServerDeploy)>;

#[test]
fn unsafe_specs_are_refused() {
    let cases: Vec<(&str, Edit)> = vec![
        (
            "root user",
            Box::new(|s| s.services[0].user = "0:0".to_owned()),
        ),
        (
            "non numeric user",
            Box::new(|s| s.services[0].user = "root".to_owned()),
        ),
        (
            "volume path traversal",
            Box::new(|s| s.services[0].volumes[0].path = "/data/../etc".to_owned()),
        ),
        (
            "relative writable path",
            Box::new(|s| s.services[0].writable = vec!["tmp".to_owned()]),
        ),
        (
            "file outside every volume",
            Box::new(|s| s.files[0].path = "/etc/passwd".to_owned()),
        ),
        (
            "file escaping its volume",
            Box::new(|s| s.files[0].path = "/data/../etc/passwd".to_owned()),
        ),
        (
            "no memory limit",
            Box::new(|s| s.services[0].limits.memory_bytes = 0),
        ),
        (
            "invalid ready pattern",
            Box::new(|s| s.lifecycle.ready = Some(ReadySpec::Console("(".to_owned()))),
        ),
        (
            "unknown lifecycle service",
            Box::new(|s| s.lifecycle.service = "ghost".to_owned()),
        ),
        (
            "privileged host port",
            Box::new(|s| {
                s.services[0].ports.push(PortSpec {
                    name: "web".to_owned(),
                    container_port: 80,
                    protocol: PortProtocol::Tcp,
                    published: Some(PublishedPort {
                        host_ip: "0.0.0.0".to_owned(),
                        host_port: 80,
                    }),
                });
            }),
        ),
        (
            "dependency cycle",
            Box::new(|s| {
                let mut db = service("db", "sleep 1");
                db.depends_on = vec!["app".to_owned()];
                s.services[0].depends_on = vec!["db".to_owned()];
                s.services.push(db);
            }),
        ),
        (
            "duplicate service",
            Box::new(|s| s.services.push(service("app", "sleep 1"))),
        ),
        (
            "environment key with an equals sign",
            Box::new(|s| {
                s.services[0]
                    .env
                    .insert("JAVA=OPTS".to_owned(), "-Xmx1G".to_owned());
            }),
        ),
        (
            "environment key starting with a digit",
            Box::new(|s| {
                s.services[0].env.insert("1A".to_owned(), "x".to_owned());
            }),
        ),
        (
            "environment value with a newline",
            Box::new(|s| {
                s.services[0].env.insert(
                    "MOTD".to_owned(),
                    "hello
PATH=/tmp"
                        .to_owned(),
                );
            }),
        ),
    ];

    for (name, edit) in cases {
        let mut spec = single(Uuid::now_v7());
        edit(&mut spec);
        assert!(check(&spec).is_err(), "{name}: accepted");
    }
}
