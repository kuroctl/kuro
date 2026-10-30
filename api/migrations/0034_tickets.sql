-- Tickets à usage unique de console et de téléchargement de sauvegarde (ADR-0014, ADR-0018),
-- en base pour être consommables par une autre instance que celle qui les a délivrés
-- (ADR-0037). Hachés, courts (30 s) ; les lignes expirées sont purgées à chaque délivrance.
CREATE TABLE tickets (
    token_hash text        PRIMARY KEY,
    server_id  uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    -- NULL : ticket de console ; sinon ticket de téléchargement de cette sauvegarde.
    backup_id  uuid        REFERENCES backups (id) ON DELETE CASCADE,
    admin_id   uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
