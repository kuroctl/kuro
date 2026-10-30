//! Supervision des process de dev : sorties préfixées, redémarrage d'un process (rechargement à
//! chaud), arrêt groupé.

use std::collections::HashMap;
use std::process::{ExitStatus, Stdio};

use anyhow::Context;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use crate::workspace::root;

#[derive(Debug, Clone, Copy)]
pub enum Color {
    Cyan,
    Magenta,
    Green,
}

impl Color {
    fn code(self) -> &'static str {
        match self {
            Self::Cyan => "36",
            Self::Magenta => "35",
            Self::Green => "32",
        }
    }
}

/// Fin d'un process supervisé.
#[derive(Debug)]
pub struct Exit {
    pub name: &'static str,
    /// Lancement auquel appartient cette fin : celle d'un process remplacé est ignorée.
    generation: u64,
    status: std::io::Result<ExitStatus>,
}

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.status {
            Ok(status) => write!(f, "{} exited ({status})", self.name),
            Err(error) => write!(f, "{} failed: {error}", self.name),
        }
    }
}

#[derive(Debug)]
struct Running {
    generation: u64,
    stop: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
}

#[derive(Debug)]
pub struct Supervisor {
    running: HashMap<&'static str, Running>,
    next_generation: u64,
    exits_tx: mpsc::UnboundedSender<Exit>,
    exits: mpsc::UnboundedReceiver<Exit>,
}

impl Supervisor {
    pub fn new() -> Self {
        let (exits_tx, exits) = mpsc::unbounded_channel();
        Self {
            running: HashMap::new(),
            next_generation: 0,
            exits_tx,
            exits,
        }
    }

    /// Lance `name`. Un process du même nom doit d'abord être arrêté par [`Self::stop`].
    pub fn spawn(
        &mut self,
        name: &'static str,
        color: Color,
        mut command: Command,
    ) -> anyhow::Result<()> {
        if command.as_std().get_current_dir().is_none() {
            command.current_dir(root());
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {name}"))?;

        let prefix = format!("\x1b[{}m[{name:<6}]\x1b[0m", color.code());
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(forward(stdout, prefix.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(forward(stderr, prefix));
        }

        self.next_generation += 1;
        let generation = self.next_generation;
        let (stop, stopped) = oneshot::channel();
        let (finished, done) = oneshot::channel();
        let exits = self.exits_tx.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                status = child.wait() => status,
                _ = stopped => {
                    let _ = child.kill().await;
                    child.wait().await
                }
            };
            let _ = finished.send(());
            let _ = exits.send(Exit {
                name,
                generation,
                status,
            });
        });
        self.running.insert(
            name,
            Running {
                generation,
                stop,
                done,
            },
        );
        Ok(())
    }

    /// Arrête `name` et attend sa fin (son port est libéré) ; sans effet s'il ne tourne pas.
    pub async fn stop(&mut self, name: &'static str) {
        if let Some(running) = self.running.remove(name) {
            let _ = running.stop.send(());
            let _ = running.done.await;
        }
    }

    fn is_current(&self, exit: &Exit) -> bool {
        self.running
            .get(exit.name)
            .is_some_and(|running| running.generation == exit.generation)
    }

    /// Process arrêté de lui-même (pas remplacé), s'il y en a un.
    pub fn exited(&mut self) -> Option<Exit> {
        while let Ok(exit) = self.exits.try_recv() {
            if self.is_current(&exit) {
                self.running.remove(exit.name);
                return Some(exit);
            }
        }
        None
    }

    /// Prochain process arrêté de lui-même ; les fins des process remplacés sont ignorées.
    pub async fn next_exit(&mut self) -> Exit {
        loop {
            // Le superviseur garde un émetteur : le canal ne se ferme jamais.
            let Some(exit) = self.exits.recv().await else {
                return std::future::pending().await;
            };
            if self.is_current(&exit) {
                self.running.remove(exit.name);
                return exit;
            }
        }
    }

    /// Arrête tous les process encore lancés.
    pub async fn shutdown(&mut self) {
        let names: Vec<&'static str> = self.running.keys().copied().collect();
        for name in names {
            self.stop(name).await;
        }
    }
}

async fn forward(stream: impl AsyncRead + Unpin, prefix: String) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        println!("{prefix} {line}");
    }
}
