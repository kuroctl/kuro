-- Accès SFTP aux fichiers des serveurs (ADR-0047), tranche 1 : clés SSH des comptes.

-- Identifiants du nom d'utilisateur SFTP (`<serveur>.<compte>`) : aléatoires, uniques, non
-- secrets. Valeur par défaut évaluée ligne à ligne, y compris pour les lignes existantes.
ALTER TABLE servers
    ADD COLUMN sftp_handle text NOT NULL UNIQUE
        DEFAULT substr(md5(random()::text || clock_timestamp()::text), 1, 10);
ALTER TABLE admin_users
    ADD COLUMN sftp_handle text NOT NULL UNIQUE
        DEFAULT substr(md5(random()::text || clock_timestamp()::text), 1, 10);

-- Port SFTP et empreinte de la clé d'hôte annoncés par le daemon dans `hello`.
ALTER TABLE nodes
    ADD COLUMN sftp_port     integer CHECK (sftp_port BETWEEN 1 AND 65535),
    ADD COLUMN sftp_host_key text;

-- Clés SSH d'un compte admin ; `customer_id` viendra avec l'espace client.
CREATE TABLE sftp_keys (
    id           uuid        PRIMARY KEY,
    admin_id     uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    name         text        NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    algorithm    text        NOT NULL,
    -- Clé publique au format du protocole SSH, en base64, sans commentaire.
    public_key   text        NOT NULL,
    -- `SHA256:…`.
    fingerprint  text        NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    UNIQUE (admin_id, fingerprint)
);

-- Événements d'audit remontés par les nœuds, déjà enregistrés : un lot renvoyé après une
-- coupure n'est pas journalisé deux fois. Purgé après 7 jours.
CREATE TABLE sftp_audit_receipts (
    event_id    uuid        PRIMARY KEY,
    received_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sftp_audit_receipts_received_at_idx ON sftp_audit_receipts (received_at);

-- Interrupteur global, coupé par défaut comme le MCP.
ALTER TABLE platform_settings ADD COLUMN sftp_enabled boolean NOT NULL DEFAULT false;
