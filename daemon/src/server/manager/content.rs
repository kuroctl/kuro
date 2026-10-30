//! Contenu des serveurs : mods, plugins (ADR-0020). L'API choisit les fichiers ; le daemon les
//! télécharge (HTTPS, adresses publiques, sha512 vérifié), les dépose sous le dossier de
//! l'emplacement par l'accès sûr aux volumes, les renomme ou les supprime. Un fichier est écrit
//! sous un nom temporaire puis renommé : un mod à moitié téléchargé n'est jamais chargé.
//!
//! Un modpack dépose ses fichiers sous le dossier du jeu (`root`, chemins relatifs) et en extrait
//! les dossiers `overrides/` et `server-overrides/` par l'extraction confinée de l'ADR-0017.
//!
//! En mode `extract` (ADR-0030, add-ons Bedrock), chaque archive est extraite à part, découpée en
//! paquets (un dossier qui porte le fichier de description), et chaque paquet rangé dans le dossier
//! que sa description désigne ; descriptions et dossiers sont rapportés dans `content.done`.

use kuro_protocol::api::server::ServerDeploy;
use kuro_protocol::api::{ContentExtract, ContentInstall, ContentRemove, ContentToggle};
use kuro_protocol::daemon::{
    ContentDone, ContentOutcome, ExtractedPackage, FileKind, MAX_DESCRIPTION_BYTES,
};
use kuro_protocol::limits::path_segments;

use super::disk::ensure_active;
use super::install::segments;
use super::{CommandError, Manager, owner_of};
use crate::fs::volumes::{Owner, VolumeError};
use crate::install::extract::Budget;
use crate::install::{InstallError, download, extract};

/// Dossiers de l'archive d'un modpack copiés dans le dossier du jeu, dans cet ordre.
const OVERRIDES: [&str; 2] = ["overrides", "server-overrides"];

/// Suffixe d'un fichier désactivé : le jeu ne le charge pas.
pub const DISABLED_SUFFIX: &str = ".disabled";
/// Archives imbriquées extraites à leur tour (`.mcpack` dans un `.mcaddon`).
const NESTED_ARCHIVES: [&str; 3] = [".mcpack", ".mcaddon", ".zip"];
/// Profondeur de recherche des paquets, et paquets au plus par archive.
const MAX_PACKAGE_DEPTH: usize = 4;
const MAX_PACKAGES: usize = 64;

/// Extraction en paquets, chemins déjà validés en chemins de volume.
struct Extraction {
    describe: Vec<String>,
    /// (route, dossier de la route en chemin de volume).
    routes: Vec<(kuro_protocol::api::ExtractRoute, Vec<String>)>,
}

impl Manager {
    /// Télécharge les fichiers dans le dossier de l'emplacement ; acceptée tout de suite,
    /// terminée par `content.done`.
    pub async fn content_install(&self, install: ContentInstall) -> Result<(), CommandError> {
        let deploy = self.spec(install.server_id)?;
        ensure_active(&deploy)?;
        let dir = content_dir(&deploy, &install.service, &install.dir)?;
        let root = install
            .root
            .as_deref()
            .map(|root| game_dir(&deploy, &install.service, root))
            .transpose()?;
        if install.archive.is_some() && root.is_none() {
            return Err(CommandError::InvalidSpec(
                "an archive needs a root".to_owned(),
            ));
        }
        let extraction = install
            .extract
            .as_ref()
            .map(|extract| extraction(&deploy, &install.service, extract, root.is_some()))
            .transpose()?;
        for file in install.files.iter().chain(&install.archive) {
            if install.archive.as_ref() != Some(file) {
                target(&dir, root.as_deref(), &file.file_name)?;
            }
            if !file.url.starts_with("https://") || !file.hash.is_well_formed() {
                return Err(CommandError::InvalidSpec(format!(
                    "invalid file {}",
                    file.file_name
                )));
            }
        }
        let total = install
            .files
            .iter()
            .chain(&install.archive)
            .map(|f| f.size)
            .sum();
        self.check_quota(&deploy, total).await?;
        let owner = owner(&deploy, &install.service)?;
        let budget = self.extraction_budget(&deploy);
        let guard = self.acquire(install.server_id)?;
        let this = self.clone();
        tokio::spawn(async move {
            // Installations de contenu limitées en nombre sur l'hôte.
            let _slot = this.inner.content_slots.clone().acquire_owned().await;
            let (outcome, packages) = match this
                .run_content_install(
                    &install,
                    &dir,
                    root.as_deref(),
                    extraction.as_ref(),
                    owner,
                    budget,
                )
                .await
            {
                Ok(packages) => (ContentOutcome::Ok, packages),
                Err(error) => {
                    tracing::warn!(server_id = %install.server_id, %error, "content install failed");
                    let outcome = ContentOutcome::Failed {
                        reason: error.to_string(),
                    };
                    (outcome, Vec::new())
                }
            };
            drop(guard);
            let done = ContentDone {
                server_id: install.server_id,
                operation_id: install.operation_id,
                outcome,
                packages,
            };
            // Lien coupé : l'API passera l'opération en échec à la reconnexion.
            let _ = this.inner.content.send(done).await;
            this.measure_later(install.server_id);
        });
        Ok(())
    }

    async fn run_content_install(
        &self,
        install: &ContentInstall,
        dir: &[String],
        root: Option<&[String]>,
        extraction: Option<&Extraction>,
        owner: Owner,
        mut budget: Budget,
    ) -> Result<Vec<ExtractedPackage>, InstallError> {
        let volumes = self.volumes(install.server_id);
        let (created, path) = (volumes.clone(), dir.to_vec());
        blocking(move || Ok(created.mkdir_all(&borrowed(&path), owner)?)).await?;
        for file in &install.files {
            let target = target(dir, root, &file.file_name)
                .map_err(|e| InstallError::Failed(e.to_string()))?;
            let (created, parent) = (volumes.clone(), target[..target.len() - 1].to_vec());
            blocking(move || Ok(created.mkdir_all(&borrowed(&parent), owner)?)).await?;
            let size = self
                .download(
                    volumes.clone(),
                    target,
                    owner,
                    &file.url,
                    download::Expected::of(&file.hash),
                )
                .await?;
            self.install_log(
                install.server_id,
                format!("{}: {size} bytes, checksum verified", file.file_name),
            );
        }
        if let (Some(archive), Some(root)) = (&install.archive, root) {
            self.extract_overrides(install, archive, root, owner, &mut budget)
                .await?;
        }
        let mut packages = Vec::new();
        if let Some(extraction) = extraction {
            for (index, file) in install.files.iter().enumerate() {
                let found = self
                    .extract_packages(
                        install,
                        extraction,
                        dir,
                        (index, &file.file_name),
                        owner,
                        &mut budget,
                    )
                    .await?;
                packages.extend(found);
            }
        }
        Ok(packages)
    }

    /// Extrait une archive téléchargée en paquets rangés d'après leur description ; l'archive et
    /// le dossier temporaire disparaissent, même en cas d'échec.
    async fn extract_packages(
        &self,
        install: &ContentInstall,
        extraction: &Extraction,
        dir: &[String],
        (index, file_name): (usize, &str),
        owner: Owner,
        budget: &mut Budget,
    ) -> Result<Vec<ExtractedPackage>, InstallError> {
        let volumes = self.volumes(install.server_id);
        let mut archive = dir.to_vec();
        archive.push(file_name.to_owned());
        let mut staging = dir.to_vec();
        staging.push(format!(".kuro-extract-{}-{index}", install.operation_id));
        let describe = extraction.describe.clone();
        let routes = extraction.routes.clone();
        let stem = package_stem(file_name);
        let source = file_name.to_owned();
        let (work, cleanup) = (volumes.clone(), (archive.clone(), staging.clone()));
        let mut left = *budget;
        let outcome = blocking(move || {
            extract::extract(
                &work,
                &borrowed(&archive),
                &borrowed(&staging),
                None,
                &[],
                owner,
                &mut left,
            )?;
            let mut found = Vec::new();
            find_packages(&work, &staging, &describe, owner, 0, &mut found, &mut left)?;
            if found.is_empty() {
                return Err(InstallError::Failed(format!(
                    "{source}: no package with {} in the archive",
                    describe.join("/")
                )));
            }
            let mut packages = Vec::with_capacity(found.len());
            for (n, package) in found.iter().enumerate() {
                let mut path = package.clone();
                path.extend(describe.iter().cloned());
                let description = read_description(&work, &path)
                    .map_err(|e| InstallError::Failed(format!("{source}: {e}")))?;
                let Some((route, route_dir)) = routes.iter().find(|(route, _)| {
                    kuro_protocol::api::matches(&description, &route.value, &route.is)
                }) else {
                    return Err(InstallError::Failed(format!(
                        "{source}: no route for package {}",
                        n + 1
                    )));
                };
                let name = format!("{stem}-{}", n + 1);
                work.mkdir_all(&borrowed(route_dir), owner)?;
                let mut target = route_dir.clone();
                target.push(name.clone());
                // Réinstallation : le paquet précédent du même nom laisse la place.
                match work.delete(&borrowed(&target)) {
                    Ok(()) | Err(VolumeError::NotFound) => {}
                    Err(error) => return Err(error.into()),
                }
                work.rename(&borrowed(package), &borrowed(&target))?;
                packages.push(ExtractedPackage {
                    file_name: source.clone(),
                    dir: route.dir.clone(),
                    name,
                    description,
                });
            }
            Ok((packages, left))
        })
        .await;
        let _ = blocking(move || {
            for path in [&cleanup.0, &cleanup.1] {
                match volumes.delete(&borrowed(path)) {
                    Ok(()) | Err(VolumeError::NotFound) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(())
        })
        .await;
        let (packages, left) = outcome?;
        *budget = left;
        self.install_log(
            install.server_id,
            format!("{file_name}: {} package(s) installed", packages.len()),
        );
        Ok(packages)
    }

    /// Télécharge l'archive du modpack dans le dossier du jeu, en extrait les `overrides`, puis
    /// la retire, même en cas d'échec.
    async fn extract_overrides(
        &self,
        install: &ContentInstall,
        archive: &kuro_protocol::api::ContentFile,
        root: &[String],
        owner: Owner,
        budget: &mut Budget,
    ) -> Result<(), InstallError> {
        let volumes = self.volumes(install.server_id);
        let mut path = root.to_vec();
        path.push(format!(".kuro-pack-{}.mrpack", install.operation_id));
        self.download(
            volumes.clone(),
            path.clone(),
            owner,
            &archive.url,
            download::Expected::of(&archive.hash),
        )
        .await?;
        let (reader, from, to) = (volumes.clone(), path.clone(), root.to_vec());
        let mut left = *budget;
        let outcome = blocking(move || {
            let mut total = extract::Extracted::default();
            for subdir in OVERRIDES {
                let done = extract::extract(
                    &reader,
                    &borrowed(&from),
                    &borrowed(&to),
                    Some(subdir),
                    &[],
                    owner,
                    &mut left,
                )?;
                total.files += done.files;
                total.bytes += done.bytes;
                total.skipped += done.skipped;
            }
            Ok((total, left))
        })
        .await;
        let _ = blocking(move || match volumes.delete(&borrowed(&path)) {
            Ok(()) | Err(VolumeError::NotFound) => Ok(()),
            Err(error) => Err(error.into()),
        })
        .await;
        let (extracted, left) = outcome?;
        *budget = left;
        self.install_log(
            install.server_id,
            format!(
                "modpack overrides: {} files ({} bytes)",
                extracted.files, extracted.bytes
            ),
        );
        Ok(())
    }

    /// Supprime des fichiers de l'emplacement, actifs ou désactivés ; un fichier absent est
    /// ignoré.
    pub async fn content_remove(&self, remove: ContentRemove) -> Result<(), CommandError> {
        let deploy = self.spec(remove.server_id)?;
        let dir = content_dir(&deploy, &remove.service, &remove.dir)?;
        let root = remove
            .root
            .as_deref()
            .map(|root| game_dir(&deploy, &remove.service, root))
            .transpose()?;
        let targets = remove
            .file_names
            .iter()
            .map(|name| target(&dir, root.as_deref(), name))
            .collect::<Result<Vec<_>, _>>()?;
        // Paquets extraits : des dossiers sous un volume du service, jamais sa racine.
        let dirs = remove
            .dirs
            .iter()
            .map(|path| content_dir(&deploy, &remove.service, path))
            .collect::<Result<Vec<_>, _>>()?;
        let guard = self.acquire(remove.server_id)?;
        let volumes = self.volumes(remove.server_id);
        let outcome = super::files::blocking(move || {
            for target in &targets {
                let (parent, name) = target.split_at(target.len() - 1);
                for candidate in [name[0].clone(), format!("{}{DISABLED_SUFFIX}", name[0])] {
                    let mut path = borrowed(parent);
                    path.push(&candidate);
                    match volumes.delete(&path) {
                        Ok(()) | Err(VolumeError::NotFound) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            for dir in &dirs {
                match volumes.delete(&borrowed(dir)) {
                    Ok(()) | Err(VolumeError::NotFound) => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })
        .await;
        drop(guard);
        self.measure_later(remove.server_id);
        outcome
    }

    /// Active (`x.jar`) ou désactive (`x.jar.disabled`) un fichier ; déjà dans l'état demandé :
    /// sans effet.
    pub async fn content_toggle(&self, toggle: ContentToggle) -> Result<(), CommandError> {
        let deploy = self.spec(toggle.server_id)?;
        ensure_active(&deploy)?;
        let dir = content_dir(&deploy, &toggle.service, &toggle.dir)?;
        file_name(&toggle.file_name)?;
        let guard = self.acquire(toggle.server_id)?;
        let volumes = self.volumes(toggle.server_id);
        let (on, off) = (
            toggle.file_name.clone(),
            format!("{}{DISABLED_SUFFIX}", toggle.file_name),
        );
        let (from, to) = if toggle.enabled { (off, on) } else { (on, off) };
        let outcome = super::files::blocking(move || {
            let mut source = borrowed(&dir);
            source.push(&from);
            let mut target = borrowed(&dir);
            target.push(&to);
            match volumes.rename(&source, &target) {
                // Déjà dans l'état demandé.
                Err(VolumeError::NotFound)
                    if volumes.list(&borrowed(&dir))?.iter().any(|e| e.name == to) =>
                {
                    Ok(())
                }
                outcome => outcome,
            }
        })
        .await;
        drop(guard);
        outcome
    }
}

/// Dossier de l'emplacement en chemin de volume (`[service, volume, …]`).
fn content_dir(
    deploy: &ServerDeploy,
    service: &str,
    dir: &str,
) -> Result<Vec<String>, CommandError> {
    let path = segments(deploy, service, dir).map_err(|_| CommandError::InvalidPath)?;
    // Sous un volume, jamais sa racine : on ne dépose pas de mod à côté du monde.
    if path.len() < 3 {
        return Err(CommandError::InvalidPath);
    }
    Ok(path)
}

/// Extraction validée : description relative, routes sous les volumes du service.
fn extraction(
    deploy: &ServerDeploy,
    service: &str,
    extract: &ContentExtract,
    modpack: bool,
) -> Result<Extraction, CommandError> {
    let describe: Vec<String> = path_segments(&extract.describe)
        .filter(|parts| !parts.is_empty() && !extract.describe.starts_with('/'))
        .ok_or(CommandError::InvalidPath)?
        .iter()
        .map(|part| (*part).to_owned())
        .collect();
    if modpack || extract.routes.is_empty() {
        return Err(CommandError::InvalidSpec(
            "an extraction needs routes and no modpack root".to_owned(),
        ));
    }
    let routes = extract
        .routes
        .iter()
        .map(|route| Ok((route.clone(), content_dir(deploy, service, &route.dir)?)))
        .collect::<Result<_, CommandError>>()?;
    Ok(Extraction { describe, routes })
}

/// Nom de base des paquets d'une archive : son nom sans extension, en lettres, chiffres, `-`, `_`.
fn package_stem(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim_matches('-');
    if stem.is_empty() {
        "pack".to_owned()
    } else {
        stem.chars().take(64).collect()
    }
}

/// Paquets sous `dir` : un dossier qui porte le fichier de description (on ne descend pas en
/// dessous) ; une archive imbriquée est extraite à côté d'elle, puis explorée.
fn find_packages(
    volumes: &crate::fs::volumes::Volumes,
    dir: &[String],
    describe: &[String],
    owner: Owner,
    depth: usize,
    found: &mut Vec<Vec<String>>,
    budget: &mut Budget,
) -> Result<(), InstallError> {
    let mut path = dir.to_vec();
    path.extend(describe.iter().cloned());
    if volumes.read(&borrowed(&path)).is_ok() {
        found.push(dir.to_vec());
        if found.len() > MAX_PACKAGES {
            return Err(InstallError::Failed(format!(
                "more than {MAX_PACKAGES} packages in the archive"
            )));
        }
        return Ok(());
    }
    if depth >= MAX_PACKAGE_DEPTH {
        return Ok(());
    }
    for entry in volumes.list(&borrowed(dir))? {
        let mut child = dir.to_vec();
        child.push(entry.name.clone());
        let nested = entry.kind == FileKind::File
            && NESTED_ARCHIVES
                .iter()
                .any(|ext| entry.name.to_ascii_lowercase().ends_with(ext));
        if entry.kind == FileKind::Directory {
            find_packages(volumes, &child, describe, owner, depth + 1, found, budget)?;
        } else if nested {
            let mut unpacked = dir.to_vec();
            unpacked.push(format!("{}.d", entry.name));
            // Même budget que l'archive qui la contient : pas d'amplification par imbrication.
            extract::extract(
                volumes,
                &borrowed(&child),
                &borrowed(&unpacked),
                None,
                &[],
                owner,
                budget,
            )?;
            find_packages(
                volumes,
                &unpacked,
                describe,
                owner,
                depth + 1,
                found,
                budget,
            )?;
        }
    }
    Ok(())
}

/// Description d'un paquet : JSON borné, sans marque d'ordre d'octets.
fn read_description(
    volumes: &crate::fs::volumes::Volumes,
    path: &[String],
) -> Result<serde_json::Value, String> {
    let file = volumes
        .read(&borrowed(path))
        .map_err(|e| format!("description: {e}"))?;
    if file.content.len() > MAX_DESCRIPTION_BYTES {
        return Err(format!(
            "description larger than {MAX_DESCRIPTION_BYTES} bytes"
        ));
    }
    serde_json::from_str(file.content.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("invalid description: {e}"))
}

/// Dossier du jeu (modpack), en chemin de volume ; la racine d'un volume est permise.
fn game_dir(deploy: &ServerDeploy, service: &str, root: &str) -> Result<Vec<String>, CommandError> {
    segments(deploy, service, root).map_err(|_| CommandError::InvalidPath)
}

/// Chemin de volume d'un fichier : un nom dans `dir`, ou un chemin relatif à `root`.
fn target(
    dir: &[String],
    root: Option<&[String]>,
    name: &str,
) -> Result<Vec<String>, CommandError> {
    let Some(root) = root else {
        file_name(name)?;
        let mut path = dir.to_vec();
        path.push(name.to_owned());
        return Ok(path);
    };
    let parts = path_segments(name).ok_or(CommandError::InvalidPath)?;
    // Relatif : un chemin absolu n'a pas sa place dans un index de modpack.
    let valid = !parts.is_empty()
        && !name.starts_with('/')
        && parts.iter().all(|p| !p.starts_with(".kuro"))
        && !name.ends_with(DISABLED_SUFFIX);
    if !valid {
        return Err(CommandError::InvalidPath);
    }
    let mut path = root.to_vec();
    path.extend(parts.iter().map(|p| (*p).to_owned()));
    // Jamais la racine d'un volume elle-même.
    if path.len() < 3 {
        return Err(CommandError::InvalidPath);
    }
    Ok(path)
}

/// Nom de fichier : un seul segment, ni caché ni désactivé.
fn file_name(name: &str) -> Result<(), CommandError> {
    let valid = path_segments(name).is_some_and(|s| s.len() == 1)
        && !name.starts_with('.')
        && !name.ends_with(DISABLED_SUFFIX);
    if valid {
        Ok(())
    } else {
        Err(CommandError::InvalidPath)
    }
}

fn owner(deploy: &ServerDeploy, service: &str) -> Result<Owner, CommandError> {
    let (uid, gid) =
        owner_of(deploy, service).map_err(|e| CommandError::InvalidSpec(e.to_string()))?;
    Ok(Owner { uid, gid })
}

fn borrowed(path: &[String]) -> Vec<&str> {
    path.iter().map(String::as_str).collect()
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, InstallError> + Send + 'static,
) -> Result<T, InstallError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| InstallError::Failed(e.to_string()))?
}
