//! Connexion SSH : authentification par clé (décidée par l'API), puis le seul sous-système
//! `sftp`. Ni shell, ni `exec`, ni redirection de port, ni agent.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use kuro_protocol::daemon::sftp::{SftpAuth, SftpCredential};
use kuro_protocol::limits::MAX_SSH_CLIENT_VERSION_BYTES;
use russh::server::{Auth, ChannelOpenHandle, Msg, PublicKeyOffer, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, Disconnect};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use uuid::Uuid;

use super::client::{Call, DaemonClient};
use super::fs::SftpFs;
use super::sessions::{Live, SessionInfo, Sessions};
use crate::fs::volumes::Volumes;
use crate::sftp::ipc::{AuthOutcome, BridgeMessage};
use crate::sftp::vfs::Tree;

/// Délai pour s'authentifier après la connexion.
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
/// Canaux ouverts au plus par connexion (FileZilla ouvre une connexion par transfert).
const MAX_CHANNELS: usize = 4;

/// Dépendances partagées par toutes les connexions.
#[derive(Debug, Clone)]
pub struct Shared {
    pub client: DaemonClient,
    pub sessions: Sessions,
    pub data_root: PathBuf,
}

/// Nom d'utilisateur de la bonne forme (`<serveur>.<compte>`) ; sinon refusé sans demander à
/// l'API, ni journaliser : ce sont les robots qui essaient `root`.
fn well_formed(username: &str) -> bool {
    let handle = |part: &str| {
        !part.is_empty()
            && part.len() <= 32
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    username
        .split_once('.')
        .is_some_and(|(server, account)| handle(server) && handle(account))
}

/// Session accordée, et ce qu'il faut pour servir le SFTP.
struct Granted {
    live: Arc<Live>,
    tree: Tree,
    history: kuro_protocol::api::HistoryPolicy,
}

pub struct Connection {
    shared: Shared,
    session_id: Uuid,
    peer: Option<SocketAddr>,
    client_version: Arc<Mutex<Vec<u8>>>,
    granted: Arc<Mutex<Option<Arc<Live>>>>,
    tree: Option<Tree>,
    /// Rétention de l'historique, reçue avec l'accord de session.
    history: kuro_protocol::api::HistoryPolicy,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl Connection {
    fn client_version(&self) -> String {
        let line = self
            .client_version
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let text = String::from_utf8_lossy(&line);
        let mut text = text.trim_end().to_owned();
        text.truncate(
            (0..=MAX_SSH_CLIENT_VERSION_BYTES.min(text.len()))
                .rev()
                .find(|i| text.is_char_boundary(*i))
                .unwrap_or(0),
        );
        text
    }

    fn live(&self) -> Option<Arc<Live>> {
        self.granted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error(transparent)]
    Russh(#[from] russh::Error),
}

impl russh::server::Handler for Connection {
    type Error = SshError;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        // Mot de passe SFTP : tranche 3 (ADR-0047).
        Ok(Auth::reject())
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        _offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if well_formed(user) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        if !well_formed(user) || self.live().is_some() {
            return Ok(Auth::reject());
        }
        let Ok(blob) = key.public_key.to_bytes() else {
            return Ok(Auth::reject());
        };
        let client_ip = self
            .peer
            .map(|peer| peer.ip().to_string())
            .unwrap_or_default();
        let client_version = self.client_version();
        let auth = SftpAuth {
            session_id: self.session_id,
            username: user.to_owned(),
            credential: SftpCredential::Publickey {
                algorithm: key.public_key.algorithm().as_str().to_owned(),
                key: STANDARD.encode(blob),
            },
            client_ip: client_ip.clone(),
            client_version: client_version.clone(),
        };
        let grant = match self.shared.client.call(Call::Auth(auth)).await {
            Ok(BridgeMessage::Auth {
                outcome: AuthOutcome::Granted(grant),
                ..
            }) => grant,
            Ok(BridgeMessage::Auth {
                outcome: AuthOutcome::Denied { reason },
                ..
            }) => {
                tracing::info!(?reason, "sftp authentication refused");
                return Ok(Auth::reject());
            }
            Ok(_) | Err(_) => {
                tracing::warn!("sftp authentication unavailable");
                return Ok(Auth::reject());
            }
        };
        let live = self.shared.sessions.open(
            SessionInfo {
                session_id: self.session_id,
                server_id: grant.server_id,
                principal: grant.principal,
                credential: grant.credential,
                client_ip,
                client_version,
                lease: Duration::from_secs(u64::from(grant.lease_secs.max(1))),
            },
            grant.write,
        );
        self.tree = Some(Tree::new(grant.layout));
        self.history = grant.history;
        *self.granted.lock().unwrap_or_else(|e| e.into_inner()) = Some(live);
        tracing::info!(server_id = %grant.server_id, "sftp session opened");
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.live().is_none() || self.channels.len() >= MAX_CHANNELS {
            // Lâché sans réponse : refusé.
            return Ok(());
        }
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let (Some(live), Some(tree)) = (self.live(), self.tree.clone()) else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        let Some(channel) = (name == "sftp")
            .then(|| self.channels.remove(&channel_id))
            .flatten()
        else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        let granted = Granted {
            live,
            tree,
            history: self.history,
        };
        let root = self
            .shared
            .data_root
            .join(granted.live.info.server_id.to_string());
        let fs = SftpFs::new(
            granted.tree,
            Volumes::new(&root),
            crate::fs::history::History::new(&root).with_policy(granted.history),
            granted.live,
            self.shared.client.clone(),
        );
        russh_sftp::server::run(channel.into_stream(), fs).await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        session.close(channel)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)?;
        session.close(channel)?;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }
}

/// Copie la ligne d'identification du client (`SSH-2.0-…`) au passage, pour l'audit.
struct Sniff<S> {
    inner: S,
    line: Arc<Mutex<Vec<u8>>>,
    done: bool,
}

impl<S: AsyncRead + Unpin> AsyncRead for Sniff<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(cx, buf);
        if !self.done {
            let fresh = buf.filled()[before..].to_vec();
            let line = Arc::clone(&self.line);
            let mut line = line.lock().unwrap_or_else(|e| e.into_inner());
            for byte in fresh {
                if byte == b'\n' || line.len() >= 255 {
                    self.done = true;
                    break;
                }
                line.push(byte);
            }
        }
        poll
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Sniff<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Sert une connexion jusqu'à sa fin : délai d'authentification, fermeture demandée (bail,
/// suspension, arrêt), puis résumé de la session journalisé.
pub async fn serve<S>(
    config: Arc<russh::server::Config>,
    shared: Shared,
    stream: S,
    peer: Option<SocketAddr>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let client_version = Arc::new(Mutex::new(Vec::new()));
    let granted: Arc<Mutex<Option<Arc<Live>>>> = Arc::new(Mutex::new(None));
    let connection = Connection {
        shared: shared.clone(),
        session_id: Uuid::now_v7(),
        peer,
        client_version: Arc::clone(&client_version),
        granted: Arc::clone(&granted),
        tree: None,
        history: kuro_protocol::api::HistoryPolicy::default(),
        channels: HashMap::new(),
    };
    let stream = Sniff {
        inner: stream,
        line: client_version,
        done: false,
    };
    let running = match russh::server::run_stream(config, stream, connection).await {
        Ok(running) => running,
        Err(error) => {
            tracing::debug!(%error, "ssh handshake failed");
            return;
        }
    };
    let handle = running.handle();
    tokio::pin!(running);
    let current = || granted.lock().unwrap_or_else(|e| e.into_inner()).clone();

    // Authentification dans le délai.
    let deadline = tokio::time::sleep(AUTH_TIMEOUT);
    tokio::pin!(deadline);
    let live = loop {
        tokio::select! {
            outcome = &mut running => {
                if let Err(error) = outcome {
                    tracing::debug!(%error, "ssh session ended");
                }
                if let Some(live) = current() {
                    shared.sessions.finish(&live);
                }
                return;
            }
            () = &mut deadline => match current() {
                Some(live) => break live,
                None => {
                    let _ = handle
                        .disconnect(Disconnect::ByApplication, "authentication timeout".to_owned(), "en".to_owned())
                        .await;
                    return;
                }
            },
            () = tokio::time::sleep(Duration::from_millis(200)) => {
                if let Some(live) = current() {
                    break live;
                }
            }
        }
    };

    tokio::select! {
        outcome = &mut running => {
            if let Err(error) = outcome {
                tracing::debug!(%error, "ssh session ended");
            }
        }
        () = live.cancel.cancelled() => {
            let _ = handle
                .disconnect(Disconnect::ByApplication, "session closed by the server".to_owned(), "en".to_owned())
                .await;
            // Le client a quelques secondes pour constater la fermeture.
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut running).await;
        }
    }
    shared.sessions.finish(&live);
    tracing::info!(server_id = %live.info.server_id, "sftp session closed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_of_the_wrong_form_never_reach_the_api() {
        assert!(well_formed("a1b2c3d4.k7f9x2mq"));
        assert!(!well_formed("root"));
        assert!(!well_formed("admin.Root"));
        assert!(!well_formed("a.b.c"));
        assert!(!well_formed(".b"));
    }
}
