-- Codes de secours du TOTP (ADR-0007) : à usage unique, remplacent un code TOTP à la connexion
-- ou pour gérer son TOTP depuis le compte. Seul un HMAC-SHA256 (clé dérivée de
-- `auth.secret_key`) est gardé : la base seule ne permet pas de les retrouver.
CREATE TABLE admin_recovery_codes (
    user_id    uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    code_hash  bytea       NOT NULL,
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, code_hash)
);
