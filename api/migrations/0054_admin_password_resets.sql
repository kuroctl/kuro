-- Mot de passe oublié (ADR-0049) : lien à usage unique envoyé par e-mail. Seule l'empreinte du
-- jeton est stockée ; une demande consommée ou remplacée garde sa trace. Une seule demande
-- ouverte par compte.
CREATE TABLE admin_password_resets (
    id          uuid        PRIMARY KEY,
    user_id     uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    token_hash  text        NOT NULL UNIQUE,
    expires_at  timestamptz NOT NULL,
    used_at     timestamptz,
    revoked_at  timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX admin_password_resets_open ON admin_password_resets (user_id)
    WHERE used_at IS NULL AND revoked_at IS NULL;
