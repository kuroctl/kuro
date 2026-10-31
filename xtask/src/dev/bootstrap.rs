//! Initialisation de l'environnement de dev, idempotente : chaque étape déjà faite est sautée.
//!
//! - PKI de l'API (`kuro-api pki init`, ne régénère jamais l'existant) ;
//! - compte super-admin, mot de passe généré et rangé dans `.kuro-dev/admin.txt` (ignoré par
//!   git, jamais affiché) ;
//! - région `eu-west` et nœud `dev-1` avec des ports (25565-25600), enrôlé dans `dev-node/` une
//!   fois l'API lancée ; un `dev-node/` dont le nœud a été révoqué ou supprimé est mis de côté
//!   et réenrôlé.
//!
//! Mêmes cas d'usage que la CLI et le REST : rien n'est réécrit ici.

use std::fmt;
use std::process::Stdio;

use anyhow::{Context, bail};
use kuro_api::audit::domain::AuditContext;
use kuro_api::auth::opaque;
use kuro_api::config::Config;
use kuro_api::db;
use kuro_api::enrollment::application::{AddNodeError, NodeTicket, add_node, reissue_token};
use kuro_api::nodes::application::{
    NewRegion, NodesError, PlacementInput, allocations, inventory, lifecycle, regions,
};
use kuro_api::nodes::domain::{CertFingerprint, NoLiveLinks, NodeStatus};
use kuro_api::nodes::infra::NodeRepository;
use kuro_api::pki::infra::PkiFiles;
use kuro_daemon::enroll::{EnrollOptions, enroll as enroll_node};
use sqlx::PgPool;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::{API_CONFIG, step};
use crate::workspace::{self, binary, root};

const NODE_DIR: &str = "dev-node";
pub const NODE_CONFIG: &str = "dev-node/daemon.toml";
const NODE_CERT: &str = "dev-node/node.pem";
/// Ancien `dev-node/` dont le nœud n'est plus valide, gardé pour inspection.
const STALE_NODE_DIR: &str = ".kuro-dev/stale-dev-node";
const ADMIN_FILE: &str = ".kuro-dev/admin.txt";
const ADMIN_EMAIL: &str = "admin@example.com";
const REGION: &str = "eu-west";
const NODE: &str = "dev-1";
/// Ports de `dev-1` (Minecraft, ARK, web…).
const DEV_PORTS: &str = "25565-25600";

/// Ce que le lancement doit encore faire après l'initialisation.
#[derive(Debug)]
pub struct Prepared {
    /// Où trouver les identifiants admin.
    pub admin_hint: String,
    /// Enrôlement du nœud de dev, à faire une fois l'API lancée.
    pub enrollment: Option<PendingEnrollment>,
}

pub struct PendingEnrollment {
    api_url: String,
    ca_fingerprint: String,
    token: String,
}

/// Le token n'apparaît jamais, même en debug.
impl fmt::Debug for PendingEnrollment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingEnrollment")
            .field("api_url", &self.api_url)
            .finish_non_exhaustive()
    }
}

pub async fn prepare(config: &Config, setup: bool) -> anyhow::Result<Prepared> {
    step("checking the dev PKI");
    let mut pki = Command::new(binary("kuro-api"));
    pki.args(["--config", API_CONFIG, "pki", "init"])
        .args(["--name", "localhost", "--name", "127.0.0.1"])
        .stdout(Stdio::null());
    workspace::run(pki, "kuro-api pki init").await?;

    let pool = db::connect(&config.database)
        .await
        .context("connecting to the dev database")?;
    db::migrate(&pool).await.context("running migrations")?;

    let admin_hint = if setup {
        "none yet (--setup): run the setup in the panel or `kuro-api setup`".to_owned()
    } else {
        ensure_admin(&pool).await?
    };
    let enrollment = if node_is_enrolled(&pool).await? {
        None
    } else {
        step("registering the dev node");
        Some(node_ticket(&pool, config).await?)
    };
    ensure_ports(&pool).await?;
    Ok(Prepared {
        admin_hint,
        enrollment,
    })
}

/// Plage de ports de `dev-1`, créée s'il n'en a aucune : de quoi déployer des serveurs tout de
/// suite (les ports existants ne sont jamais touchés).
async fn ensure_ports(pool: &PgPool) -> anyhow::Result<()> {
    let Ok(node_id) = inventory::id_by_name(pool, NODE).await else {
        return Ok(());
    };
    if allocations::list(pool, node_id).await?.is_empty() {
        step(&format!("adding ports {DEV_PORTS} to {NODE}"));
        allocations::create(
            pool,
            &AuditContext::cli("xtask dev"),
            node_id,
            None,
            DEV_PORTS,
        )
        .await?;
    }
    Ok(())
}

/// `dev-node/` utilisable : son certificat désigne un nœud connu et non révoqué, comme le
/// vérifie l'API au handshake mTLS. Un `dev-node/` périmé est mis de côté.
async fn node_is_enrolled(pool: &PgPool) -> anyhow::Result<bool> {
    let cert_path = root().join(NODE_CERT);
    if !root().join(NODE_CONFIG).exists() || !cert_path.exists() {
        return Ok(false);
    }
    let certs =
        kuro_runtime::tls::load_certs(&cert_path).context("reading the dev node certificate")?;
    let node = match certs.first() {
        Some(cert) => {
            NodeRepository::new(pool.clone())
                .find_by_fingerprint(&CertFingerprint::from_der(cert))
                .await?
        }
        None => None,
    };
    if node.is_some_and(|node| node.status != NodeStatus::Revoked) {
        return Ok(true);
    }

    step(&format!(
        "{NODE_DIR}/ refers to a revoked or deleted node: moved to {STALE_NODE_DIR}/"
    ));
    let stale = root().join(STALE_NODE_DIR);
    if stale.exists() {
        std::fs::remove_dir_all(&stale)?;
    }
    if let Some(parent) = stale.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(root().join(NODE_DIR), &stale)
        .with_context(|| format!("moving {NODE_DIR}/ aside"))?;
    Ok(false)
}

/// Enrôle le nœud de dev auprès de l'API lancée : clé générée localement, comme en production.
pub async fn enroll(pending: PendingEnrollment) -> anyhow::Result<()> {
    let enrolled = enroll_node(&EnrollOptions {
        api_url: pending.api_url,
        ca_fingerprint: pending.ca_fingerprint,
        token: pending.token,
        config_path: root().join(NODE_CONFIG),
    })
    .await
    .context("enrolling the dev node")?;
    step(&format!(
        "node {} enrolled ({})",
        enrolled.node_name, enrolled.node_id
    ));
    Ok(())
}

/// Crée le super-admin de dev tant que la base n'a aucun compte (base neuve ou remise à zéro,
/// même si `admin.txt` existe encore) ; un compte existant n'est jamais modifié.
async fn ensure_admin(pool: &PgPool) -> anyhow::Result<String> {
    let file = root().join(ADMIN_FILE);
    let hint = format!("{ADMIN_EMAIL}, credentials in {ADMIN_FILE}");
    let accounts: i64 = sqlx::query_scalar("SELECT count(*) FROM admin_users")
        .fetch_one(pool)
        .await
        .context("counting admin accounts")?;
    if accounts > 0 {
        return Ok(hint);
    }

    step("creating the dev admin account");
    let password: String = opaque::generate("")?.chars().take(24).collect();
    let mut child = Command::new(binary("kuro-api"))
        .args([
            "--config",
            API_CONFIG,
            "admin",
            "create",
            "--password-stdin",
        ])
        .args(["--email", ADMIN_EMAIL, "--name", "Admin"])
        .current_dir(root())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting kuro-api admin create")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(format!("{password}\n").as_bytes()).await?;
    }
    let output = child.wait_with_output().await?;
    let stderr = String::from_utf8_lossy(&output.stderr);

    let content = if output.status.success() {
        format!(
            "# Compte admin de dev créé par `cargo xtask dev` (fichier ignoré par git).\n\
             email: {ADMIN_EMAIL}\npassword: {password}\n"
        )
    } else if stderr.contains("email_taken") {
        format!(
            "# Le compte {ADMIN_EMAIL} existait déjà : mot de passe inchangé.\n\
             # Pour en imposer un : cargo run -p kuro-api -- --config {API_CONFIG} \
             admin reset-password --email {ADMIN_EMAIL}\nemail: {ADMIN_EMAIL}\n"
        )
    } else {
        bail!("kuro-api admin create failed: {}", stderr.trim());
    };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, content).with_context(|| format!("writing {ADMIN_FILE}"))?;
    Ok(hint)
}

/// Région et nœud de dev, puis un token d'enrôlement neuf.
async fn node_ticket(pool: &PgPool, config: &Config) -> anyhow::Result<PendingEnrollment> {
    let context = AuditContext::cli("xtask dev");
    let region_id = match regions::id_by_slug(pool, REGION).await {
        Ok(id) => id,
        Err(NodesError::UnknownRegion) => {
            let region = NewRegion {
                slug: REGION.to_owned(),
                name: "Europe Ouest".to_owned(),
                latitude: 48.85,
                longitude: 2.35,
            };
            regions::create(pool, &context, region).await?.id
        }
        Err(error) => return Err(error.into()),
    };

    let ttl = config.enrollment.token_ttl();
    let placement = PlacementInput {
        region_id: Some(region_id),
        display_name: Some("Dev 1".to_owned()),
        location: None,
        coordinates: None,
    };
    let ticket: NodeTicket = match add_node(pool, &context, NODE, placement, ttl).await {
        Ok(ticket) => ticket,
        // Nœud déjà connu (dossier `dev-node/` supprimé) : on le réenrôle.
        Err(AddNodeError::AlreadyExists) => {
            let id = inventory::id_by_name(pool, NODE).await?;
            match reissue_token(pool, &context, id, ttl).await {
                Ok(ticket) => ticket,
                Err(_) => {
                    let reason = Some("re-enrolled by cargo xtask dev".to_owned());
                    lifecycle::revoke(pool, &context, &NoLiveLinks, id, reason).await?;
                    reissue_token(pool, &context, id, ttl).await?
                }
            }
        }
        Err(error) => return Err(error.into()),
    };

    let ca_fingerprint = PkiFiles::from(&config.daemon)
        .load_ca()
        .context("loading the dev CA")?
        .fingerprint();
    Ok(PendingEnrollment {
        api_url: format!("https://127.0.0.1:{}", config.daemon.bind.port()),
        ca_fingerprint: format!("sha256:{ca_fingerprint}"),
        token: ticket.token.expose().to_owned(),
    })
}
