//! Arborescence présentée à une session SFTP (ADR-0047).
//!
//! - Un seul service avec un seul volume : ce volume est la racine (`/server.properties`).
//! - Sinon : `/<service>/<volume>/…`, comme l'explorateur du panel ; seuls les services et
//!   volumes de la spécification déployée existent.
//!
//! Les chemins reçus sont normalisés **lexicalement** (`.` et `..` résolus sans toucher au disque,
//! `..` à la racine reste à la racine) : aucune résolution ne sort de la racine virtuelle. Chaque
//! nom doit ensuite être acceptable ([`kuro_protocol::limits::is_valid_name`]), et l'accès réel
//! passe par [`crate::fs::volumes::Volumes`], qui ne suit aucun lien.

use kuro_protocol::limits::is_valid_name;

use super::ipc::ServiceLayout;

/// Où mène un chemin SFTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// Racine virtuelle : la liste des services.
    Root,
    /// Dossier d'un service : la liste de ses volumes.
    Service(String),
    /// Chemin réel sous `volumes/` : `[service, volume, …]`.
    Real(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Tree {
    services: Vec<ServiceLayout>,
}

impl Tree {
    /// Seuls les services qui ont des volumes sont visibles.
    pub fn new(layout: Vec<ServiceLayout>) -> Self {
        Self {
            services: layout
                .into_iter()
                .filter(|service| !service.volumes.is_empty())
                .collect(),
        }
    }

    /// Le volume unique du serveur, présenté comme racine.
    fn single(&self) -> Option<(&str, &str)> {
        match self.services.as_slice() {
            [service] => match service.volumes.as_slice() {
                [volume] => Some((service.name.as_str(), volume.as_str())),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn services(&self) -> impl Iterator<Item = &str> {
        self.services.iter().map(|service| service.name.as_str())
    }

    pub fn volumes(&self, service: &str) -> Option<&[String]> {
        self.services
            .iter()
            .find(|s| s.name == service)
            .map(|s| s.volumes.as_slice())
    }

    pub fn owner(&self, service: &str) -> Option<(u32, u32)> {
        self.services
            .iter()
            .find(|s| s.name == service)
            .map(|s| (s.uid, s.gid))
    }

    /// Chemin à montrer au client pour un chemin reçu : absolu, normalisé.
    pub fn canonical(path: &str) -> String {
        let segments = normalize(path);
        format!("/{}", segments.join("/"))
    }

    /// `None` : chemin refusé (nom interdit) ou hors des services et volumes déclarés.
    pub fn locate(&self, path: &str) -> Option<Location> {
        let segments = normalize(path);
        if !segments.iter().all(|segment| is_valid_name(segment)) {
            return None;
        }
        if let Some((service, volume)) = self.single() {
            let mut real = vec![service.to_owned(), volume.to_owned()];
            real.extend(segments);
            return Some(Location::Real(real));
        }
        match segments.as_slice() {
            [] => Some(Location::Root),
            [service] => self
                .volumes(service)
                .map(|_| Location::Service(service.clone())),
            [service, volume, ..] => self
                .volumes(service)
                .filter(|volumes| volumes.contains(volume))
                .map(|_| Location::Real(segments)),
        }
    }
}

/// Segments d'un chemin, `.` et `..` résolus sans sortir de la racine.
fn normalize(path: &str) -> Vec<String> {
    let mut segments: Vec<String> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            name => segments.push(name.to_owned()),
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, volumes: &[&str]) -> ServiceLayout {
        ServiceLayout {
            name: name.to_owned(),
            volumes: volumes.iter().map(|v| (*v).to_owned()).collect(),
            uid: 1000,
            gid: 1000,
        }
    }

    fn real(segments: &[&str]) -> Option<Location> {
        Some(Location::Real(
            segments.iter().map(|s| (*s).to_owned()).collect(),
        ))
    }

    #[test]
    fn a_single_volume_is_the_root() {
        let tree = Tree::new(vec![service("server", &["data"]), service("idle", &[])]);
        assert_eq!(tree.locate("/"), real(&["server", "data"]));
        assert_eq!(tree.locate(""), real(&["server", "data"]));
        assert_eq!(
            tree.locate("/world/level.dat"),
            real(&["server", "data", "world", "level.dat"])
        );
        assert_eq!(
            tree.locate("server.properties"),
            real(&["server", "data", "server.properties"])
        );
    }

    #[test]
    fn several_volumes_show_services_then_volumes() {
        let tree = Tree::new(vec![
            service("web", &["html", "config"]),
            service("db", &["data"]),
        ]);
        assert_eq!(tree.locate("/"), Some(Location::Root));
        assert_eq!(
            tree.locate("/web"),
            Some(Location::Service("web".to_owned()))
        );
        assert_eq!(
            tree.locate("/web/html/index.php"),
            real(&["web", "html", "index.php"])
        );
        assert_eq!(tree.locate("/db/data"), real(&["db", "data"]));
        // Ni service ni volume hors de la spécification.
        assert_eq!(tree.locate("/cache"), None);
        assert_eq!(tree.locate("/web/logs/x"), None);
    }

    #[test]
    fn dot_dot_never_leaves_the_root() {
        let tree = Tree::new(vec![service("server", &["data"])]);
        assert_eq!(
            tree.locate("/../../etc/passwd"),
            real(&["server", "data", "etc", "passwd"])
        );
        assert_eq!(tree.locate("/a/../../b"), real(&["server", "data", "b"]));
        assert_eq!(Tree::canonical("/../a/./b/.."), "/a");
        assert_eq!(Tree::canonical("."), "/");

        let tree = Tree::new(vec![service("web", &["html"]), service("db", &["data"])]);
        assert_eq!(
            tree.locate("/web/html/../../db/data/x"),
            real(&["db", "data", "x"])
        );
        assert_eq!(tree.locate("/web/html/../../../.."), Some(Location::Root));
    }

    #[test]
    fn forbidden_names_are_refused() {
        let tree = Tree::new(vec![service("server", &["data"])]);
        assert_eq!(tree.locate("/a\\b"), None);
        assert_eq!(tree.locate("/a\u{0}b"), None);
        assert_eq!(tree.locate(&format!("/{}", "x".repeat(256))), None);
    }
}
