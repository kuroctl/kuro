//! Émission des jetons : session (accès + rafraîchissement) et challenges de connexion
//! (mot de passe à remplacer, TOTP).

use std::fmt;

use sqlx::PgConnection;
use uuid::Uuid;

use super::{IamError, IamState};
use crate::auth::{jwt, opaque};
use crate::iam::infra::SessionRepository;

/// Audience des jetons d'accès de l'espace admin.
pub const ADMIN_AUDIENCE: &str = "kuro-admin";
const REFRESH_PREFIX: &str = "kuro_rt_";
const CHALLENGE_TTL_SECS: u64 = 5 * 60;

/// Étape restante après un mot de passe valide.
#[derive(Debug, Clone, Copy)]
pub(super) enum ChallengePurpose {
    /// Mot de passe choisi par un autre admin, à remplacer avant le TOTP.
    ChangePassword,
    VerifyTotp,
    SetupTotp,
}

impl ChallengePurpose {
    /// Une audience par usage : un challenge ne sert ni de jeton d'accès, ni à l'autre étape.
    fn audience(self) -> &'static str {
        match self {
            Self::ChangePassword => "kuro-admin-password-change",
            Self::VerifyTotp => "kuro-admin-totp-verify",
            Self::SetupTotp => "kuro-admin-totp-setup",
        }
    }
}

pub struct SessionTokens {
    pub access_token: String,
    pub expires_in: u64,
    /// À transmettre uniquement dans un cookie `HttpOnly`.
    pub refresh_token: String,
}

impl fmt::Debug for SessionTokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionTokens")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// Jeton d'accès seul, sans nouveau jeton de rafraîchissement.
pub struct AccessToken {
    pub access_token: String,
    pub expires_in: u64,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub struct Challenge {
    pub token: String,
    pub expires_in: u64,
}

pub(super) fn now() -> u64 {
    u64::try_from(time::OffsetDateTime::now_utc().unix_timestamp()).unwrap_or(0)
}

/// Ouvre une session (nouvelle famille de jetons de rafraîchissement).
pub(super) async fn open_session(
    conn: &mut PgConnection,
    state: &IamState,
    user_id: Uuid,
) -> Result<SessionTokens, IamError> {
    issue(conn, state, user_id, Uuid::now_v7()).await
}

/// Émet un jeton de rafraîchissement dans la famille `family_id` et le jeton d'accès associé.
pub(super) async fn issue(
    conn: &mut PgConnection,
    state: &IamState,
    user_id: Uuid,
    family_id: Uuid,
) -> Result<SessionTokens, IamError> {
    let refresh_token = opaque::generate(REFRESH_PREFIX)?;
    SessionRepository::insert(
        conn,
        family_id,
        user_id,
        &opaque::hash(&refresh_token),
        state.settings.refresh_ttl,
    )
    .await?;
    let AccessToken {
        access_token,
        expires_in,
    } = access(state, user_id, family_id);
    Ok(SessionTokens {
        access_token,
        expires_in,
        refresh_token,
    })
}

/// Jeton d'accès rattaché à la session `family_id`.
pub(super) fn access(state: &IamState, user_id: Uuid, family_id: Uuid) -> AccessToken {
    let ttl = state.settings.access_ttl.as_secs();
    let claims = jwt::Claims::new(ADMIN_AUDIENCE, user_id, Some(family_id), now(), ttl);
    AccessToken {
        access_token: jwt::encode(state.keys.jwt(), &claims),
        expires_in: ttl,
    }
}

pub(super) fn challenge(state: &IamState, user_id: Uuid, purpose: ChallengePurpose) -> Challenge {
    let claims = jwt::Claims::new(purpose.audience(), user_id, None, now(), CHALLENGE_TTL_SECS);
    Challenge {
        token: jwt::encode(state.keys.jwt(), &claims),
        expires_in: CHALLENGE_TTL_SECS,
    }
}

pub(super) fn read_challenge(
    state: &IamState,
    token: &str,
    purpose: ChallengePurpose,
) -> Result<Uuid, IamError> {
    jwt::decode(state.keys.jwt(), token, purpose.audience(), now())
        .map(|claims| claims.sub)
        .map_err(|_| IamError::InvalidChallenge)
}
