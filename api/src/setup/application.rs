//! Cas d'usage de la mise en route (ADR-0045) : état « à configurer », jeton sur l'hôte, création
//! du super admin et du nom du projet. Le REST et la CLI appellent les mêmes fonctions ; seule
//! l'autorité change (`Admin` jamais ici : la route est anonyme, la CLI est `Host`).

use std::path::{Path, PathBuf};

use serde_json::json;
use sqlx::PgPool;

use crate::audit::domain::{AuditAction, AuditContext, AuditEvent};
use crate::audit::infra::AuditedTx;
use crate::auth::opaque;
use crate::error_code::{ErrorCode, FieldError};
use crate::iam::application::{AdminUser, IamError, IamState, NewAdminUser, users};
use crate::iam::domain::{Authority, SUPER_ADMIN_ROLE_ID};
use crate::iam::infra::UserRepository;
use crate::settings::infra::SettingsRepository;

/// Préfixe du jeton : reconnaissable, et détectable par les scanners de secrets.
const TOKEN_PREFIX: &str = "kuro_setup_";
/// Verrou consultatif Postgres : deux mises en route concurrentes, une seule passe.
const LOCK_KEY: &str = "kuro_setup";

#[derive(Debug, Clone)]
pub struct SetupState {
    pub pool: PgPool,
    pub iam: IamState,
    /// Fichier du jeton de mise en route (`setup.token_file`), lu à chaque `POST`.
    pub token_file: PathBuf,
}

/// Ce que tout le monde peut savoir : l'API attend-elle son premier compte, et comment s'appelle
/// le projet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStatus {
    pub needed: bool,
    pub project_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SetupRequest {
    pub email: String,
    pub display_name: String,
    pub password: String,
    pub project_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// Un compte existe déjà : la mise en route est passée, pour toujours.
    #[error("setup already done")]
    Done,
    #[error("invalid setup token")]
    InvalidToken,
    /// Le fichier du jeton n'a pas pu être écrit au démarrage : passer par la CLI.
    #[error("setup token unavailable: {0}")]
    TokenUnavailable(String),
    #[error("invalid field")]
    Invalid(FieldError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for SetupError {
    fn from(error: sqlx::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<IamError> for SetupError {
    fn from(error: IamError) -> Self {
        match error {
            IamError::Invalid(field) => Self::Invalid(field),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl SetupState {
    pub async fn status(&self) -> Result<SetupStatus, sqlx::Error> {
        let mut conn = self.pool.acquire().await?;
        let needed = UserRepository::count_all(&mut conn).await? == 0;
        let project_name = SettingsRepository::get(&mut conn).await?.project_name;
        Ok(SetupStatus {
            needed,
            project_name,
        })
    }

    /// Au démarrage : écrit le jeton si l'API est à configurer et qu'il n'existe pas encore.
    /// Rend le chemin du fichier quand un jeton est en place.
    pub async fn ensure_token(&self) -> Result<Option<&Path>, SetupError> {
        if !self.status().await?.needed {
            return Ok(None);
        }
        if !self.token_file.exists() {
            let token = opaque::generate(TOKEN_PREFIX)
                .map_err(|error| SetupError::TokenUnavailable(error.to_string()))?;
            write_private(&self.token_file, &token)
                .map_err(|error| SetupError::TokenUnavailable(error.to_string()))?;
        }
        Ok(Some(&self.token_file))
    }

    /// Jeton attendu, lu sur l'hôte ; absent si le fichier n'a pas été écrit.
    fn expected_token(&self) -> Result<String, SetupError> {
        std::fs::read_to_string(&self.token_file)
            .map(|text| text.trim().to_owned())
            .map_err(|error| SetupError::TokenUnavailable(error.to_string()))
    }

    /// Met l'API en route : premier compte (super admin) et nom du projet. `token` : le jeton
    /// reçu par HTTP, vérifié contre le fichier de l'hôte ; `None` depuis la CLI, où l'accès au
    /// shell vaut tous les droits. Dans les deux cas le cas d'usage agit avec l'autorité `Host`.
    pub async fn run(
        &self,
        token: Option<&str>,
        context: &AuditContext,
        request: SetupRequest,
    ) -> Result<AdminUser, SetupError> {
        let authority = &Authority::Host;
        let project_name = match request.project_name.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(name) if name.chars().count() <= 80 && !name.chars().any(char::is_control) => {
                Some(name.to_owned())
            }
            Some(_) => {
                return Err(SetupError::Invalid(FieldError::new(
                    "project_name",
                    ErrorCode::InvalidProjectName,
                )));
            }
        };

        // Le verrou tient pendant toute la mise en route : un second appel attend, puis voit le
        // compte et reçoit « déjà fait ».
        let mut lock = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
            .bind(LOCK_KEY)
            .execute(&mut *lock)
            .await?;
        if UserRepository::count_all(&mut lock).await? > 0 {
            return Err(SetupError::Done);
        }
        if let Some(given) = token {
            let expected = self.expected_token()?;
            // Les empreintes se comparent à longueur fixe : rien du secret ne fuit par le temps.
            if opaque::hash(given) != opaque::hash(&expected) {
                return Err(SetupError::InvalidToken);
            }
        }

        let user = users::create(
            &self.iam,
            authority,
            context,
            NewAdminUser {
                email: request.email,
                display_name: request.display_name,
                password: request.password,
            },
        )
        .await?;
        users::set_roles(
            &self.iam,
            authority,
            context,
            user.id,
            &[SUPER_ADMIN_ROLE_ID],
        )
        .await
        .map_err(|error| {
            SetupError::Internal(format!(
                "account {} created but super-admin role not granted: {error}",
                user.id
            ))
        })?;

        let mut tx = AuditedTx::begin(&self.pool, context).await?;
        let mut settings = SettingsRepository::lock(&mut tx).await?;
        settings.project_name = project_name.clone();
        SettingsRepository::save(&mut tx, &settings).await?;
        tx.commit(
            AuditEvent::success(AuditAction::SetupCompleted)
                .target("admin_user", user.id)
                .details(json!({ "project_name": project_name })),
        )
        .await?;
        lock.commit().await?;

        // Le jeton a servi ; un fichier qui ne part pas n'ouvre plus rien (le compte existe).
        if self.token_file.exists()
            && let Err(error) = std::fs::remove_file(&self.token_file)
        {
            tracing::warn!(%error, path = %self.token_file.display(), "setup token file not removed");
        }
        Ok(user)
    }
}

/// Écrit le jeton lisible par le seul propriétaire.
fn write_private(path: &Path, token: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        writeln!(file, "{token}")?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, format!("{token}\n"))
    }
}
