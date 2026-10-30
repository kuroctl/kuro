//! Serveur HTTPS local pour les tests d'installation : certificat auto-signé pour 127.0.0.1,
//! fichiers servis par chemin, redirections.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use kuro_daemon::install::download::DownloadPolicy;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

pub enum Route {
    File(Vec<u8>),
    Redirect(String),
}

pub struct TestServer {
    pub addr: SocketAddr,
    pub policy: DownloadPolicy,
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("https://{}{path}", self.addr)
    }
}

/// Sert `routes` en HTTPS ; la politique retournée fait confiance au certificat et autorise
/// l'adresse locale (le daemon les refuse en production).
pub async fn serve(routes: HashMap<String, Route>) -> TestServer {
    let generated = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    let cert = CertificateDer::from(generated.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        generated.signing_key.serialize_der(),
    ));
    let config = rustls::ServerConfig::builder_with_provider(kuro_runtime::tls::provider())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let (acceptor, routes) = (acceptor.clone(), routes.clone());
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let routes = routes.clone();
                    async move {
                        let response = match routes.get(request.uri().path()) {
                            Some(Route::File(bytes)) => {
                                Response::new(Full::new(Bytes::from(bytes.clone())))
                            }
                            Some(Route::Redirect(location)) => Response::builder()
                                .status(StatusCode::FOUND)
                                .header("location", location)
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                            None => Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        };
                        Ok::<_, Infallible>(response)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tls), service)
                    .await;
            });
        }
    });

    let mut roots = RootCertStore::empty();
    roots.add(cert).unwrap();
    TestServer {
        addr,
        policy: DownloadPolicy {
            roots: Arc::new(roots),
            allow_private: true,
            ..DownloadPolicy::default()
        },
    }
}
