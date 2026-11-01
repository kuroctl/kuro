//! Démarrage du serveur HTTP avec arrêt propre.

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::router_with;
use crate::config::HttpConfig;
use crate::extension::MountedExtension;
use crate::state::AppState;

pub async fn serve(
    config: &HttpConfig,
    state: AppState,
    extensions: Vec<MountedExtension>,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %listener.local_addr()?, "http server listening");
    // `ConnectInfo` : l'adresse du pair alimente le journal d'audit.
    axum::serve(
        listener,
        router_with(state, extensions)
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.cancelled_owned())
    .await
}
