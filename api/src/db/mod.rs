//! Accès PostgreSQL : pool de connexions et migrations embarquées.

use sqlx::PgPool;
use sqlx::migrate::MigrateError;
use sqlx::postgres::PgPoolOptions;

use crate::config::DatabaseConfig;

/// Pool réglé par `[database]` : taille, attente d'une connexion, recyclage des connexions.
pub async fn connect(config: &DatabaseConfig) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(config.acquire_timeout())
        .idle_timeout(config.idle_timeout())
        .max_lifetime(config.max_lifetime())
        .connect(&config.url)
        .await
}

/// Applique les migrations de `api/migrations/`, embarquées à la compilation.
pub async fn migrate(pool: &PgPool) -> Result<(), MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
