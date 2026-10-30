//! Fichiers d'un serveur sur l'hôte : `<data_root>/<server_id>/`.
//!
//! - `volumes/<service>/<volume>/` : seuls dossiers montés dans les conteneurs ;
//! - `.kuro/spec.json` : dernière spécification déployée, jamais montée.
//!
//! Un chemin du conteneur n'est accepté que sous un volume du service, sans `..` ni lien
//! symbolique sur le trajet. Les fichiers déclarés sont écrits conteneurs arrêtés : aucun
//! processus du serveur ne peut substituer un lien entre la vérification et l'écriture. Les
//! fichiers modifiés depuis le panel, serveur en marche, passent par [`volumes`] (`openat2`).

pub mod managed;
pub mod volumes;

use std::path::{Component, Path, PathBuf};

use kuro_protocol::api::server::{FileContent, FileFormat, FileSpec, ServerDeploy, ServiceSpec};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("`{0}` is not under a volume of its service")]
    OutsideVolume(String),
    #[error("`{0}` goes through a symbolic link")]
    Symlink(PathBuf),
    #[error("unknown service `{0}`")]
    UnknownService(String),
    #[error("cannot edit keys of a {0:?} file")]
    UnsupportedFormat(FileFormat),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid stored spec: {0}")]
    Spec(#[from] serde_json::Error),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> FsError + '_ {
    move |source| FsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Dossier d'un serveur sur l'hôte.
#[derive(Debug, Clone)]
pub struct ServerDir {
    root: PathBuf,
}

impl ServerDir {
    pub fn new(data_root: &Path, server_id: Uuid) -> Self {
        Self {
            root: data_root.join(server_id.to_string()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn spec_path(&self) -> PathBuf {
        self.root.join(".kuro").join("spec.json")
    }

    /// Marque d'une spécification gardée sans être appliquée (`defer`) : les conteneurs sont
    /// recréés au prochain démarrage.
    fn pending_path(&self) -> PathBuf {
        self.root.join(".kuro").join("pending")
    }

    pub fn is_pending(&self) -> bool {
        self.pending_path().exists()
    }

    pub fn set_pending(&self, pending: bool) -> Result<(), FsError> {
        let path = self.pending_path();
        if pending {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            std::fs::write(&path, b"").map_err(io(&path))
        } else {
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io(&path)(error)),
            }
        }
    }

    pub fn volume_dir(&self, service: &str, volume: &str) -> PathBuf {
        self.root.join("volumes").join(service).join(volume)
    }

    pub fn exists(&self) -> bool {
        self.spec_path().exists()
    }

    /// Écrit la spécification en entier ou pas du tout : fichier temporaire puis renommage, pour
    /// qu'un arrêt brutal en pleine écriture ne laisse jamais un serveur sans spécification.
    pub fn save_spec(&self, spec: &ServerDeploy) -> Result<(), FsError> {
        let path = self.spec_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(spec)?).map_err(io(&temp))?;
        std::fs::rename(&temp, &path).map_err(io(&path))
    }

    pub fn load_spec(&self) -> Result<Option<ServerDeploy>, FsError> {
        let path = self.spec_path();
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io(&path)(error)),
        }
    }

    /// Crée les dossiers des volumes ; retourne (service, dossier sur l'hôte) pour chacun.
    pub fn ensure_volumes(&self, spec: &ServerDeploy) -> Result<Vec<(String, PathBuf)>, FsError> {
        let mut created = Vec::new();
        for service in &spec.services {
            for volume in &service.volumes {
                let dir = self.volume_dir(&service.name, &volume.name);
                std::fs::create_dir_all(&dir).map_err(io(&dir))?;
                created.push((service.name.clone(), dir));
            }
        }
        Ok(created)
    }

    pub fn remove(&self) -> Result<(), FsError> {
        match std::fs::remove_dir_all(&self.root) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(io(&self.root)(error))
            }
            _ => Ok(()),
        }
    }

    /// Chemin sur l'hôte d'un chemin du conteneur, sous un volume du service.
    pub fn resolve(&self, service: &ServiceSpec, container_path: &str) -> Result<PathBuf, FsError> {
        let outside = || FsError::OutsideVolume(container_path.to_owned());
        let (volume, rest) = service
            .volumes
            .iter()
            .find_map(|v| {
                let root = v.path.trim_end_matches('/');
                if container_path == root {
                    Some((v, ""))
                } else {
                    container_path
                        .strip_prefix(&format!("{root}/"))
                        .map(|rest| (v, rest))
                }
            })
            .ok_or_else(outside)?;

        let base = self.volume_dir(&service.name, &volume.name);
        let mut path = base.clone();
        for part in Path::new(rest).components() {
            match part {
                Component::Normal(name) => path.push(name),
                Component::CurDir => {}
                _ => return Err(outside()),
            }
        }
        // Aucun lien symbolique entre la racine du volume et la cible.
        let mut current = base;
        for part in path
            .strip_prefix(&current)
            .map_err(|_| outside())?
            .components()
        {
            current.push(part);
            match std::fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => return Err(FsError::Symlink(current)),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(io(&current)(error)),
            }
        }
        Ok(path)
    }

    /// Écrit un fichier déclaré, selon son mode. Retourne son chemin s'il a été écrit.
    pub fn write_file(
        &self,
        spec: &ServerDeploy,
        file: &FileSpec,
    ) -> Result<Option<PathBuf>, FsError> {
        let service = spec
            .services
            .iter()
            .find(|s| s.name == file.service)
            .ok_or_else(|| FsError::UnknownService(file.service.clone()))?;
        let path = self.resolve(service, &file.path)?;
        let content = match &file.content {
            FileContent::Template { content } => content.clone(),
            FileContent::User { content } => {
                if path.exists() {
                    return Ok(None);
                }
                content.clone().unwrap_or_default()
            }
            FileContent::ManagedKeys { keys } => {
                let original = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(error) => return Err(io(&path)(error)),
                };
                match file.format {
                    FileFormat::Properties => managed::properties(&original, keys),
                    FileFormat::Ini => managed::ini(&original, keys),
                    other => return Err(FsError::UnsupportedFormat(other)),
                }
            }
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&path, content).map_err(io(&path))?;
        Ok(Some(path))
    }
}
