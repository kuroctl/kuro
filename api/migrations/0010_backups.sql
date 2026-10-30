-- Sauvegardes des serveurs (ADR-0018). Les archives restent sur le nœud ; la base garde leur
-- état, leur taille et leur empreinte. Effacées avec le serveur.
CREATE TABLE backups (
    id uuid PRIMARY KEY,
    server_id uuid NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    name text NOT NULL,
    state text NOT NULL CHECK (state IN ('creating', 'ready', 'failed')),
    size_bytes bigint,
    sha256 text,
    locked boolean NOT NULL DEFAULT false,
    failure text,
    created_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz
);
CREATE INDEX backups_server_created ON backups (server_id, created_at);
-- Au plus une sauvegarde en cours par serveur.
CREATE UNIQUE INDEX backups_one_creating ON backups (server_id) WHERE state = 'creating';

-- Sauvegardes gardées par serveur ; 0 les désactive. Au-delà, rotation des plus anciennes.
ALTER TABLE servers ADD COLUMN backup_limit integer NOT NULL DEFAULT 3
    CHECK (backup_limit BETWEEN 0 AND 100);

-- Restauration d'une sauvegarde : nouvel état rapporté par le daemon.
ALTER TABLE servers DROP CONSTRAINT servers_state_check;
ALTER TABLE servers ADD CONSTRAINT servers_state_check CHECK (state IN
    ('deploying', 'installing', 'restoring', 'offline', 'starting', 'running', 'stopping',
     'failed', 'deleting'));
