//! Opérations cryptographiques de la CA, sans I/O.

mod authority;

pub use authority::{CertificateAuthority, IssuedCertificate, PkiError, SignedCertificate};
