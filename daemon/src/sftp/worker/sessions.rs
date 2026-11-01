//! Sessions SFTP ouvertes : baux renouvelés auprès de l'API (par le daemon), fermetures, et
//! événements d'audit remontés.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kuro_protocol::daemon::sftp::{
    CloseReason, SessionSummary, SftpCredentialRef, SftpEvent, SftpEventKind, SftpLeaseRequest,
    SftpPrincipal,
};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::client::{Call, DaemonClient};
use crate::sftp::ipc::BridgeMessage;

/// Événements gardés en mémoire quand le daemon ne les accepte pas ; au-delà, les plus anciens
/// sont perdus (journalisé).
const MAX_HELD_EVENTS: usize = 10_000;
const AUDIT_RETRY: Duration = Duration::from_secs(2);

/// Ce que l'API a accordé à une session, et qui la porte.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: Uuid,
    pub server_id: Uuid,
    pub principal: SftpPrincipal,
    pub credential: SftpCredentialRef,
    pub client_ip: String,
    pub client_version: String,
    pub lease: Duration,
}

/// État partagé entre la session SSH, son sous-système SFTP et le renouvellement des baux.
#[derive(Debug)]
pub struct Live {
    pub info: SessionInfo,
    /// `servers.files.write`, mis à jour à chaque bail.
    pub write: AtomicBool,
    pub cancel: CancellationToken,
    reason: Mutex<Option<CloseReason>>,
    renewed: Mutex<Instant>,
    opened: Instant,
    pub bytes_read: AtomicU64,
    pub files_read: AtomicU64,
    pub bytes_written: AtomicU64,
    pub files_written: AtomicU64,
    pub denied: AtomicU64,
    /// Permissions déjà signalées en `access.denied` (une fois par session).
    reported: Mutex<HashSet<String>>,
    audit: AuditQueue,
}

impl Live {
    /// Ferme la session (la première raison l'emporte).
    pub fn close(&self, reason: CloseReason) {
        let mut current = self.reason.lock().unwrap_or_else(|e| e.into_inner());
        if current.is_none() {
            *current = Some(reason);
        }
        self.cancel.cancel();
    }

    /// Opération refusée (chemin, lien, permission).
    pub fn refused(&self) {
        self.denied.fetch_add(1, Ordering::Relaxed);
    }

    /// Permission manquante : comptée, et journalisée la première fois.
    pub fn forbidden(&self, permission: &str) {
        self.refused();
        let first = self
            .reported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(permission.to_owned());
        if first {
            self.audit.push(self.event(SftpEventKind::AccessDenied {
                permission: permission.to_owned(),
            }));
        }
    }

    /// Écriture faite : journalisée (`file.written`, `file.renamed`…).
    pub fn wrote(&self, event: SftpEventKind) {
        if let SftpEventKind::FileWritten { .. } = event {
            self.files_written.fetch_add(1, Ordering::Relaxed);
        }
        self.audit.push(self.event(event));
    }

    /// Les écritures sont refusées tant que le daemon n'accepte plus d'événements d'audit :
    /// aucune écriture sans trace (ADR-0047).
    pub fn audit_blocked(&self) -> bool {
        self.audit.blocked()
    }

    fn event(&self, event: SftpEventKind) -> SftpEvent {
        SftpEvent {
            event_id: Uuid::now_v7(),
            occurred_at_ms: now_ms(),
            session_id: self.info.session_id,
            server_id: self.info.server_id,
            principal: self.info.principal,
            client_ip: self.info.client_ip.clone(),
            client_version: self.info.client_version.clone(),
            event,
        }
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Sessions {
    live: Arc<Mutex<HashMap<Uuid, Arc<Live>>>>,
    audit: AuditQueue,
}

impl Sessions {
    pub fn new(audit: AuditQueue) -> Self {
        Self {
            live: Arc::default(),
            audit,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<Live>>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn open(&self, info: SessionInfo, write: bool) -> Arc<Live> {
        let live = Arc::new(Live {
            write: AtomicBool::new(write),
            cancel: CancellationToken::new(),
            reason: Mutex::new(None),
            renewed: Mutex::new(Instant::now()),
            opened: Instant::now(),
            bytes_read: AtomicU64::new(0),
            files_read: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            files_written: AtomicU64::new(0),
            denied: AtomicU64::new(0),
            reported: Mutex::new(HashSet::new()),
            audit: self.audit.clone(),
            info,
        });
        self.lock().insert(live.info.session_id, Arc::clone(&live));
        live
    }

    /// Fin de la session SSH : résumé journalisé.
    pub fn finish(&self, live: &Live) {
        self.lock().remove(&live.info.session_id);
        let reason = live
            .reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unwrap_or(CloseReason::Client);
        let summary = SessionSummary {
            duration_secs: live.opened.elapsed().as_secs(),
            reason,
            bytes_read: live.bytes_read.load(Ordering::Relaxed),
            files_read: live.files_read.load(Ordering::Relaxed),
            bytes_written: live.bytes_written.load(Ordering::Relaxed),
            files_written: live.files_written.load(Ordering::Relaxed),
            denied: live.denied.load(Ordering::Relaxed),
        };
        self.audit
            .push(live.event(SftpEventKind::SessionClosed(summary)));
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn close_server(&self, server_id: Uuid, reason: CloseReason) {
        for live in self.lock().values() {
            if live.info.server_id == server_id {
                live.close(reason);
            }
        }
    }

    pub fn close_all(&self, reason: CloseReason) {
        for live in self.lock().values() {
            live.close(reason);
        }
    }

    /// Renouvelle les baux de toutes les sessions ; une session refusée, ou dont le bail a
    /// expiré faute de réponse, est fermée.
    pub async fn renew(&self, client: &DaemonClient) {
        let sessions: Vec<Arc<Live>> = self.lock().values().cloned().collect();
        if sessions.is_empty() {
            return;
        }
        let requests = sessions
            .iter()
            .map(|live| SftpLeaseRequest {
                session_id: live.info.session_id,
                server_id: live.info.server_id,
                principal: live.info.principal,
                credential: live.info.credential,
            })
            .collect();
        let results = match client.call(Call::Lease(requests)).await {
            Ok(BridgeMessage::Lease { results, .. }) => results,
            _ => Vec::new(),
        };
        let now = Instant::now();
        for live in sessions {
            match results
                .iter()
                .find(|r| r.session_id == live.info.session_id)
            {
                Some(result) if result.granted => {
                    live.write.store(result.write, Ordering::Relaxed);
                    *live.renewed.lock().unwrap_or_else(|e| e.into_inner()) = now;
                }
                Some(_) => live.close(CloseReason::LeaseDenied),
                None => {
                    let renewed = *live.renewed.lock().unwrap_or_else(|e| e.into_inner());
                    if now.duration_since(renewed) >= live.info.lease {
                        live.close(CloseReason::LeaseDenied);
                    }
                }
            }
        }
    }

    /// Renouvellement périodique (à mi-bail) et fermetures demandées par le daemon.
    pub async fn maintain(
        self,
        client: DaemonClient,
        every: Duration,
        shutdown: CancellationToken,
    ) {
        let mut closes = client.closes();
        let mut ticker = tokio::time::interval(every);
        ticker.tick().await;
        loop {
            tokio::select! {
                () = shutdown.cancelled() => return,
                _ = ticker.tick() => self.renew(&client).await,
                close = closes.recv() => match close {
                    Ok((server_id, reason)) => self.close_server(server_id, reason),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        closes = client.closes();
                    }
                },
            }
        }
    }
}

/// Événements d'audit à remettre au daemon, dans l'ordre.
#[derive(Debug, Clone)]
pub struct AuditQueue {
    events: mpsc::UnboundedSender<SftpEvent>,
    /// Le daemon a refusé (file pleine) ou n'a pas répondu : des événements attendent.
    blocked: Arc<AtomicBool>,
}

impl AuditQueue {
    pub fn new() -> (Self, AuditReceiver) {
        let (events, receiver) = mpsc::unbounded_channel();
        let blocked = Arc::new(AtomicBool::new(false));
        (
            Self {
                events,
                blocked: Arc::clone(&blocked),
            },
            AuditReceiver { receiver, blocked },
        )
    }

    pub fn push(&self, event: SftpEvent) {
        let _ = self.events.send(event);
    }

    pub fn blocked(&self) -> bool {
        self.blocked.load(Ordering::Relaxed)
    }
}

/// Côté transfert de la file d'audit.
#[derive(Debug)]
pub struct AuditReceiver {
    receiver: mpsc::UnboundedReceiver<SftpEvent>,
    blocked: Arc<AtomicBool>,
}

/// Remet les événements au daemon par lots ; refusés (daemon absent, file pleine), ils sont
/// gardés et représentés.
pub async fn forward_audit(
    audit: AuditReceiver,
    client: DaemonClient,
    shutdown: CancellationToken,
) {
    let AuditReceiver {
        receiver: mut events,
        blocked,
    } = audit;
    let mut held: Vec<SftpEvent> = Vec::new();
    loop {
        if held.is_empty() {
            tokio::select! {
                event = events.recv() => match event {
                    Some(event) => held.push(event),
                    None => return,
                },
                () = shutdown.cancelled() => return,
            }
        }
        while let Ok(event) = events.try_recv() {
            held.push(event);
        }
        if held.len() > MAX_HELD_EVENTS {
            let lost = held.len() - MAX_HELD_EVENTS;
            tracing::error!(
                lost,
                "sftp audit events lost: the daemon does not accept them"
            );
            held.drain(..lost);
        }
        let batch: Vec<SftpEvent> = held
            .iter()
            .take(kuro_protocol::limits::MAX_SFTP_AUDIT_BATCH)
            .cloned()
            .collect();
        let count = batch.len();
        match client.call(Call::Audit(batch)).await {
            Ok(BridgeMessage::Audit { accepted: true, .. }) => {
                held.drain(..count);
                if held.is_empty() {
                    blocked.store(false, Ordering::Relaxed);
                }
            }
            _ => {
                blocked.store(true, Ordering::Relaxed);
                tokio::select! {
                    () = tokio::time::sleep(AUDIT_RETRY) => {}
                    () = shutdown.cancelled() => return,
                }
            }
        }
    }
}
