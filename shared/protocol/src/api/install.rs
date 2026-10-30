//! Installation et réinstallation d'un serveur (ADR-0017). La première installation voyage dans
//! `server.deploy` ; `server.install` réinstalle ou réinitialise. Les étapes arrivent résolues par
//! l'API (gabarits, contenu des `templates/`) ; le daemon les exécute dans l'ordre avec ses propres
//! règles (adresses publiques seulement, sha256 avant écriture, chemins sous les volumes).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInstall {
    pub server_id: Uuid,
    pub mode: InstallMode,
    /// Étapes du prefab ; absent pour un prefab qui s'installe lui-même au démarrage (un
    /// `reset` vide alors seulement les volumes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<InstallSpec>,
}

/// Plateforme des fichiers d'une app Steam (ADR-0032). Un serveur Windows tourne ensuite sous
/// Proton, fourni par l'image d'exécution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteamPlatform {
    #[default]
    Linux,
    Windows,
}

impl SteamPlatform {
    pub fn is_linux(&self) -> bool {
        *self == Self::Linux
    }
}

/// Compte Steam d'une étape `steamcmd` qui refuse la connexion anonyme (ADR-0033). Envoyé à
/// chaque installation, jamais gardé par le nœud ; `Debug` n'écrit pas le mot de passe.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamLogin {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for SteamLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SteamLogin")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    /// Étapes relancées sur les données existantes (mise à jour du jeu).
    Reinstall,
    /// Volumes vidés, puis installation.
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallSpec {
    /// Service dont les volumes reçoivent l'installation.
    pub service: String,
    /// Image du conteneur d'installation (`steamcmd`, `script`).
    pub image: String,
    pub steps: Vec<InstallStep>,
}

/// Une étape ; la forme suit le format des prefabs (`{ "download": { … } }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStep {
    Download {
        url: String,
        /// SHA-256 hexadécimal attendu, vérifié avant toute écriture.
        sha256: String,
        to: String,
    },
    Extract {
        from: String,
        to: String,
        /// Seul ce dossier de l'archive est extrait, à la racine de `to`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subdir: Option<String>,
        /// Chemins (relatifs à `to`) qui reçoivent le droit d'exécution.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        executable: Vec<String>,
    },
    /// Contenu d'un gabarit du prefab, déjà résolu.
    Copy {
        content: String,
        to: String,
    },
    Remove {
        path: String,
    },
    Steamcmd {
        app: u32,
        dir: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beta: Option<String>,
        #[serde(default)]
        validate: bool,
        /// Plateforme téléchargée (ADR-0032) : `linux` par défaut, omis alors.
        #[serde(default, skip_serializing_if = "SteamPlatform::is_linux")]
        platform: SteamPlatform,
        /// Compte de l'installation (ADR-0033) ; absent : connexion anonyme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        login: Option<SteamLogin>,
    },
    /// Porte de sortie : `sh -c <run>` dans le conteneur d'installation isolé.
    Script {
        run: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
    },
}
