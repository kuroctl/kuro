//! Navigateur simulé : REST servi sur un vrai port, WebSocket de console ouvert avec une origine
//! choisie.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

const WAIT: Duration = Duration::from_secs(10);

/// Sert le routeur sur un port local ; le serveur vit autant que le test.
pub async fn serve(router: Router) -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

pub struct Browser {
    ws: WebSocketStream<TcpStream>,
}

/// Fin de connexion constatée par le navigateur.
#[derive(Debug, PartialEq, Eq)]
pub enum Closed {
    Code(u16),
    Without,
}

impl Browser {
    /// Ouvre le WebSocket de console ; `Err(statut)` si le serveur refuse la poignée de main.
    pub async fn open(addr: SocketAddr, server_id: &str, origin: &str) -> Result<Self, u16> {
        let url = format!("ws://{addr}/api/admin/v1/servers/{server_id}/console");
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("origin", HeaderValue::from_str(origin).unwrap());
        let tcp = TcpStream::connect(addr).await.unwrap();
        match tokio_tungstenite::client_async(request, tcp).await {
            Ok((ws, _)) => Ok(Self { ws }),
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                Err(response.status().as_u16())
            }
            Err(error) => panic!("websocket handshake failed: {error}"),
        }
    }

    pub async fn send(&mut self, message: Value) {
        self.ws
            .send(Message::text(message.to_string()))
            .await
            .unwrap();
    }

    pub async fn auth(&mut self, ticket: &str) {
        self.send(serde_json::json!({ "type": "auth", "ticket": ticket }))
            .await;
    }

    /// Prochain message texte ; `Err` à la fermeture.
    pub async fn next(&mut self) -> Result<Value, Closed> {
        tokio::time::timeout(WAIT, async {
            while let Some(frame) = self.ws.next().await {
                match frame {
                    Ok(Message::Text(text)) => {
                        return Ok(serde_json::from_str(text.as_str()).unwrap());
                    }
                    Ok(Message::Close(Some(frame))) => {
                        return Err(Closed::Code(u16::from(frame.code)));
                    }
                    Ok(Message::Close(None)) | Err(_) => return Err(Closed::Without),
                    Ok(_) => {}
                }
            }
            Err(Closed::Without)
        })
        .await
        .expect("no message from the console websocket")
    }

    /// Prochain message d'un type donné, en ignorant les autres.
    pub async fn next_of(&mut self, kind: &str) -> Value {
        loop {
            let message = self.next().await.expect("console closed");
            if message["type"] == kind {
                return message;
            }
        }
    }

    /// Attend la fermeture et retourne son code.
    pub async fn closed(&mut self) -> Closed {
        loop {
            if let Err(closed) = self.next().await {
                return closed;
            }
        }
    }

    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}
