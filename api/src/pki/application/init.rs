//! Initialisation idempotente : la CA et le certificat serveur ne sont créés que s'ils manquent.
//! Relancer ne régénère jamais un secret existant.

use crate::pki::domain::CertificateAuthority;
use crate::pki::infra::{PkiFiles, PkiStoreError};

const CA_NAME: &str = "Kuro CA";

#[derive(Debug)]
pub struct InitReport {
    pub ca_created: bool,
    pub server_created: bool,
    /// Empreinte à passer à `kuro-daemonctl enroll --ca-fingerprint`.
    pub ca_fingerprint: String,
}

/// `server_names` : noms DNS ou IP sous lesquels les daemons joignent l'API.
pub fn init(files: &PkiFiles, server_names: &[String]) -> Result<InitReport, PkiStoreError> {
    let ca_created = !files.ca_exists();
    let ca = if ca_created {
        let ca = CertificateAuthority::generate(CA_NAME)?;
        files.save_ca(&ca)?;
        ca
    } else {
        files.load_ca()?
    };

    let server_created = !files.server_exists();
    if server_created {
        let server = ca.issue_server(server_names)?;
        files.save_server(&server.chain_pem, &server.key_pem)?;
    }

    Ok(InitReport {
        ca_created,
        server_created,
        ca_fingerprint: ca.fingerprint(),
    })
}
