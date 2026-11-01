//! Postgres de test : un conteneur réutilisé entre les lancements, une base neuve par test.

use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use testcontainers::{ImageExt, ReuseDirective, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::OnceCell;
use uuid::Uuid;

const DB_PREFIX: &str = "kuro_test_";

/// URL du serveur, sans nom de base.
static SERVER_URL: OnceCell<String> = OnceCell::const_new();

/// Base vierge, migrations appliquées.
pub async fn fresh_pool() -> PgPool {
    let pool = empty_pool().await;
    kuro_api::db::migrate(&pool).await.unwrap();
    pool
}

/// Base vierge, sans migration (le harnais des extensions les applique lui-même).
pub async fn empty_pool() -> PgPool {
    let server = SERVER_URL.get_or_init(start_server).await;
    let name = format!("{DB_PREFIX}{}", Uuid::now_v7().simple());

    let mut admin = PgConnection::connect(&format!("{server}/postgres"))
        .await
        .unwrap();
    sqlx::raw_sql(AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut admin)
        .await
        .unwrap();

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&format!("{server}/{name}"))
        .await
        .unwrap()
}

async fn start_server() -> String {
    // Conteneur nommé et réutilisé : un seul `kuro-test-postgres` subsiste entre deux lancements.
    let container = Postgres::default()
        .with_tag("16-alpine")
        .with_container_name("kuro-test-postgres")
        .with_reuse(ReuseDirective::Always)
        .start()
        .await
        .expect("failed to start postgres container (is Docker running?)");
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let server = format!("postgres://postgres:postgres@{host}:{port}");

    drop_stale_databases(&server).await;
    server
}

/// Les bases des lancements précédents ne sont jamais supprimées en fin de test : on nettoie ici,
/// avant qu'un test de ce lancement n'en crée.
async fn drop_stale_databases(server: &str) {
    let mut admin = PgConnection::connect(&format!("{server}/postgres"))
        .await
        .unwrap();
    let stale: Vec<String> =
        sqlx::query_scalar("SELECT datname FROM pg_database WHERE datname LIKE $1")
            .bind(format!("{DB_PREFIX}%"))
            .fetch_all(&mut admin)
            .await
            .unwrap();
    for name in stale {
        sqlx::raw_sql(AssertSqlSafe(format!("DROP DATABASE {name} WITH (FORCE)")))
            .execute(&mut admin)
            .await
            .unwrap();
    }
}
