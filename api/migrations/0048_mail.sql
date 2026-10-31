-- Envoi d'e-mails (ADR-0046) : réglages SMTP du projet, invitations des admins, canal e-mail des
-- notifications.

-- Une seule ligne : le serveur SMTP du projet. Le mot de passe est chiffré (secret_box), jamais
-- rendu. `security` : STARTTLS ou TLS implicite, jamais en clair.
CREATE TABLE mail_settings (
    singleton    boolean     PRIMARY KEY DEFAULT true CHECK (singleton),
    host         text        NOT NULL CHECK (length(host) BETWEEN 1 AND 253),
    port         integer     NOT NULL CHECK (port BETWEEN 1 AND 65535),
    security     text        NOT NULL CHECK (security IN ('starttls', 'tls')),
    username     text        CHECK (length(username) BETWEEN 1 AND 255),
    password     bytea,
    from_address text        NOT NULL CHECK (length(from_address) BETWEEN 3 AND 254),
    -- Nom affiché de l'expéditeur ; NULL : le nom du projet.
    from_name    text        CHECK (length(from_name) BETWEEN 1 AND 80),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

-- Invitation d'un admin : seule l'empreinte du jeton est stockée ; une invitation consommée ou
-- remplacée garde sa trace. Une seule invitation ouverte par compte.
CREATE TABLE admin_invitations (
    id          uuid        PRIMARY KEY,
    user_id     uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    token_hash  text        NOT NULL UNIQUE,
    invited_by  uuid        REFERENCES admin_users (id) ON DELETE SET NULL,
    expires_at  timestamptz NOT NULL,
    used_at     timestamptz,
    revoked_at  timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX admin_invitations_open ON admin_invitations (user_id)
    WHERE used_at IS NULL AND revoked_at IS NULL;

-- Canal e-mail : config chiffrée { recipients: [...] }, url_hint = première adresse masquée.
ALTER TABLE notification_channels DROP CONSTRAINT notification_channels_kind_check;
ALTER TABLE notification_channels
    ADD CONSTRAINT notification_channels_kind_check
    CHECK (kind IN ('webhook', 'discord', 'slack', 'email'));
