use kuro_protocol::enroll::{EnrollRequest, EnrollResponse};
use serde_json::json;
use uuid::Uuid;

const NODE_ID: &str = "6f1c2d3e-4a5b-4c6d-8e7f-001122334455";

#[test]
fn enroll_request_wire_format() {
    let request = EnrollRequest {
        token: "kuro_enr_secret".to_owned(),
        csr_pem: "-----BEGIN CERTIFICATE REQUEST-----".to_owned(),
    };
    let expected =
        json!({ "token": "kuro_enr_secret", "csr_pem": "-----BEGIN CERTIFICATE REQUEST-----" });

    assert_eq!(serde_json::to_value(&request).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<EnrollRequest>(expected).unwrap(),
        request
    );
}

#[test]
fn enroll_request_never_shows_its_token() {
    let request = EnrollRequest {
        token: "kuro_enr_secret".to_owned(),
        csr_pem: String::new(),
    };

    assert!(!format!("{request:?}").contains("secret"));
}

#[test]
fn enroll_response_wire_format() {
    let response = EnrollResponse {
        node_id: Uuid::parse_str(NODE_ID).unwrap(),
        node_name: "node-1".to_owned(),
        cert_pem: "cert".to_owned(),
        ca_pem: "ca".to_owned(),
    };
    let expected =
        json!({ "node_id": NODE_ID, "node_name": "node-1", "cert_pem": "cert", "ca_pem": "ca" });

    assert_eq!(serde_json::to_value(&response).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<EnrollResponse>(expected).unwrap(),
        response
    );
}
