//! Bus de notifications entre instances : `LISTEN`/`NOTIFY` de Postgres, une connexion dédiée
//! par instance. Une notification ne porte qu'un réveil (payload court) ; les données sont en
//! base. Un tic périodique sert de secours si une notification se perd (reconnexion).

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgListener;
use tokio_util::sync::CancellationToken;

/// Réaction à une notification ou à un tic : courte, elle lance une tâche si besoin.
pub type Handler = Arc<dyn Fn(&str) + Send + Sync>;
pub type Tick = Arc<dyn Fn() + Send + Sync>;

/// Délai avant de retenter l'écoute après une erreur.
const RETRY: Duration = Duration::from_secs(2);

pub struct Bus {
    pool: PgPool,
    subscriptions: Vec<(&'static str, Handler)>,
    ticks: Vec<(Duration, Tick)>,
}

impl Bus {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            subscriptions: Vec::new(),
            ticks: Vec::new(),
        }
    }

    /// Appelle `handler` avec le payload de chaque notification du canal.
    pub fn subscribe(mut self, channel: &'static str, handler: Handler) -> Self {
        self.subscriptions.push((channel, handler));
        self
    }

    /// Appelle `tick` à chaque période, notification ou pas.
    pub fn every(mut self, period: Duration, tick: Tick) -> Self {
        self.ticks.push((period, tick));
        self
    }

    /// Émet une notification sur un canal, depuis n'importe quelle connexion.
    pub async fn notify(pool: &PgPool, channel: &str, payload: &str) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(payload)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Écoute jusqu'à l'arrêt ; se reconnecte après une erreur.
    pub async fn run_forever(self, shutdown: CancellationToken) {
        let channels: Vec<&'static str> = self.subscriptions.iter().map(|(c, _)| *c).collect();
        let mut tickers: Vec<(tokio::time::Interval, Tick)> = self
            .ticks
            .iter()
            .map(|(period, tick)| {
                let mut interval = tokio::time::interval(*period);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                (interval, Arc::clone(tick))
            })
            .collect();
        loop {
            let listener = tokio::select! {
                () = shutdown.cancelled() => return,
                listener = self.connect(&channels) => listener,
            };
            let mut listener = match listener {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::warn!(%error, "cluster bus not listening, retrying");
                    tokio::select! {
                        () = shutdown.cancelled() => return,
                        () = tokio::time::sleep(RETRY) => continue,
                    }
                }
            };
            let has_ticks = !tickers.is_empty();
            loop {
                let ticks = async {
                    let fired = futures_util::future::select_all(tickers.iter_mut().map(
                        |(interval, tick)| {
                            let tick = Arc::clone(tick);
                            Box::pin(async move {
                                interval.tick().await;
                                tick
                            })
                        },
                    ))
                    .await;
                    fired.0
                };
                tokio::select! {
                    () = shutdown.cancelled() => return,
                    tick = ticks, if has_ticks => tick(),
                    notification = listener.recv() => match notification {
                        Ok(notification) => {
                            for (channel, handler) in &self.subscriptions {
                                if *channel == notification.channel() {
                                    handler(notification.payload());
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "cluster bus lost its connection, reconnecting");
                            break;
                        }
                    },
                }
            }
        }
    }

    async fn connect(&self, channels: &[&'static str]) -> Result<PgListener, sqlx::Error> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen_all(channels.iter().copied()).await?;
        Ok(listener)
    }
}
