//! Accès SFTP aux fichiers des serveurs (ADR-0047).
//!
//! Le SSH est servi par un **processus séparé** (`kuro-daemon sftp`, unité `kuro-sftp.service`) :
//! utilisateur dédié, sans socket Docker ni certificat mTLS, qui ne voit que les données des
//! serveurs. Il ne parle qu'au daemon, par un socket Unix ([`ipc`]) ; le daemon ([`bridge`])
//! relaie authentifications et baux vers l'API par le lien mTLS, et garde les événements d'audit
//! dans une file locale ([`spool`]) jusqu'à leur acquittement.
//!
//! - [`vfs`] : arborescence présentée (racine simplifiée ou `/<service>/<volume>/…`) ;
//! - [`worker`] : le serveur SSH et le sous-système SFTP, au-dessus de [`crate::fs::volumes`].

pub mod hostkey;
pub mod ipc;
pub mod spool;
pub mod vfs;

#[cfg(unix)]
pub mod bridge;
#[cfg(unix)]
pub mod worker;

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Section `[sftp]` de la configuration du nœud. Sans `bind`, le SFTP est coupé : ni socket, ni
/// clé d'hôte, et `hello` ne l'annonce pas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpConfig {
    /// Adresse d'écoute SSH du processus SFTP (`0.0.0.0:2022`).
    #[serde(default)]
    pub bind: Option<SocketAddr>,
    /// Port annoncé aux clients, s'il diffère de celui de `bind` (redirection, NAT).
    #[serde(default)]
    pub public_port: Option<u16>,
    /// Socket Unix entre le daemon et le processus SFTP.
    #[serde(default = "default_socket")]
    pub socket: PathBuf,
    /// Clé d'hôte ed25519, générée par le daemon au premier démarrage (`0600`, root). Le service
    /// SFTP la reçoit par `LoadCredential=` de systemd.
    #[serde(default = "default_host_key")]
    pub host_key: PathBuf,
    /// File des événements d'audit pas encore acquittés par l'API, hors des données des serveurs.
    #[serde(default = "default_spool")]
    pub spool: PathBuf,
    /// Seul utilisateur admis sur le socket (`SO_PEERCRED`) ; son groupe reçoit le socket en
    /// `0660`. Absent ou vide (tests, nœud de dev où tout tourne en root) : aucune vérification,
    /// socket en `0600`.
    #[serde(default = "default_worker_user")]
    pub worker_user: Option<String>,
}

impl SftpConfig {
    pub fn announced_port(&self) -> Option<u16> {
        self.bind
            .map(|bind| self.public_port.unwrap_or_else(|| bind.port()))
    }
}

impl Default for SftpConfig {
    fn default() -> Self {
        Self {
            bind: None,
            public_port: None,
            socket: default_socket(),
            host_key: default_host_key(),
            spool: default_spool(),
            worker_user: default_worker_user(),
        }
    }
}

fn default_socket() -> PathBuf {
    PathBuf::from("/run/kuro/sftp.sock")
}

fn default_host_key() -> PathBuf {
    PathBuf::from("/etc/kuro/sftp_host_ed25519")
}

fn default_spool() -> PathBuf {
    PathBuf::from("/var/lib/kuro/daemon/sftp-audit")
}

fn default_worker_user() -> Option<String> {
    Some("kuro-sftp".to_owned())
}
