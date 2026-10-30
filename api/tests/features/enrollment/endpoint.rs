//! `POST /api/enroll` appelé à la main : validation de la CSR et règles d'accès.

use hyper::StatusCode;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use serde_json::json;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::support::link::{TestApi, node_files};
use crate::support::pki::key_and_csr;

#[tokio::test]
async fn invalid_csr_is_rejected_without_consuming_the_token() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;

    let (status, body) = api
        .post_enroll(
            json!({ "token": ticket.token.expose(), "csr_pem": "not a csr" }),
            None,
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_csr");
    api.enroll_from("host-1", ticket.token.expose(), &api.pki.fingerprint)
        .await
        .unwrap();
}

#[tokio::test]
async fn csr_with_a_forged_signature_is_rejected() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    let (_, csr) = key_and_csr("node");

    let (status, body) = api
        .post_enroll(
            json!({ "token": ticket.token.expose(), "csr_pem": tamper_signature(&csr) }),
            None,
        )
        .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "invalid_csr");
}

#[tokio::test]
async fn ca_imposes_the_certificate_identity() {
    let api = TestApi::start().await;
    let ticket = api.add_node("node-1").await;
    let (_, csr) = key_and_csr("admin");

    let (status, body) = api
        .post_enroll(
            json!({ "token": ticket.token.expose(), "csr_pem": csr }),
            None,
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    let pem = body["cert_pem"].as_str().unwrap();
    let der = CertificateDer::from_pem_slice(pem.as_bytes()).unwrap();
    let (_, cert) = X509Certificate::from_der(&der).unwrap();
    let common_names: Vec<_> = cert
        .subject()
        .iter_common_name()
        .map(|cn| cn.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(common_names, ["node-1"]);
    let usages = cert.extended_key_usage().unwrap().unwrap().value;
    assert!(usages.client_auth && !usages.server_auth);
    assert!(
        cert.basic_constraints()
            .unwrap()
            .is_none_or(|bc| !bc.value.ca)
    );
}

#[tokio::test]
async fn enrolled_node_cannot_enroll_again() {
    let api = TestApi::start().await;
    let node = api.enroll_node("node-1").await;
    let ticket = api.add_node("node-2").await;
    let (_, csr) = key_and_csr("node");

    let (status, body) = api
        .post_enroll(
            json!({ "token": ticket.token.expose(), "csr_pem": csr }),
            Some(&node_files(&node)),
        )
        .await;

    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "already_enrolled");
    assert_eq!(api.status(ticket.node.id).await, "pending");
}

/// Altère un caractère de la signature (fin du PEM) sans casser l'encodage base64.
fn tamper_signature(csr_pem: &str) -> String {
    let end = csr_pem.find("-----END").unwrap();
    let body = &csr_pem[..end];
    let line_start = body.trim_end().rfind('\n').unwrap() + 1;
    // La dernière ligne base64 ne contient que des octets de la signature.
    let index = line_start;
    let original = csr_pem.as_bytes()[index];
    let replacement = if original == b'A' { 'B' } else { 'A' };
    format!(
        "{}{}{}",
        &csr_pem[..index],
        replacement,
        &csr_pem[index + 1..]
    )
}
