-- Espace admin : comptes, rôles (RBAC) et sessions. Voir ADR-0007 (audit : 0004, ADR-0008).

CREATE TABLE admin_users (
    id                  uuid        PRIMARY KEY,
    email               text        NOT NULL,
    display_name        text        NOT NULL,
    password_hash       text        NOT NULL,
    -- Secrets TOTP chiffrés (XChaCha20-Poly1305). NULL : TOTP non configuré.
    totp_secret         bytea,
    totp_pending_secret bytea,
    -- Dernier pas TOTP accepté (anti-rejeu).
    totp_last_step      bigint,
    failed_attempts     integer     NOT NULL DEFAULT 0,
    locked_until        timestamptz,
    disabled_at         timestamptz,
    last_login_at       timestamptz,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX admin_users_email_key ON admin_users (lower(email));

CREATE TABLE roles (
    id          uuid        PRIMARY KEY,
    name        text        NOT NULL UNIQUE,
    description text        NOT NULL DEFAULT '',
    -- Rôle livré par Kuro : ni modifiable ni supprimable.
    is_system   boolean     NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE role_permissions (
    role_id    uuid NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    permission text NOT NULL,
    PRIMARY KEY (role_id, permission)
);

CREATE TABLE admin_user_roles (
    user_id uuid NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    role_id uuid NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, role_id)
);

CREATE INDEX admin_user_roles_role_id_idx ON admin_user_roles (role_id);

-- Une ligne par jeton de rafraîchissement émis ; une session (`family_id`) en regroupe la chaîne de rotation.
CREATE TABLE admin_sessions (
    id                 uuid        PRIMARY KEY,
    family_id          uuid        NOT NULL,
    user_id            uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    refresh_token_hash text        NOT NULL UNIQUE,
    expires_at         timestamptz NOT NULL,
    rotated_at         timestamptz,
    revoked_at         timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX admin_sessions_family_id_idx ON admin_sessions (family_id);
CREATE INDEX admin_sessions_user_id_idx ON admin_sessions (user_id);

INSERT INTO roles (id, name, description, is_system)
VALUES ('00000000-0000-7000-8000-000000000001', 'super-admin', 'Toutes les permissions, y compris futures.', true);

INSERT INTO role_permissions (role_id, permission)
VALUES ('00000000-0000-7000-8000-000000000001', '*');
