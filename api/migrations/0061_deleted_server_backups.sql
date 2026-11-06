-- Sauvegardes d'un serveur supprimé (ADR-0059) : gardées par son nœud `deleted_backups_days`
-- jours (0 : effacées avec lui), visibles de l'admin, reprises par un autre serveur du nœud.

ALTER TABLE platform_settings
    ADD COLUMN deleted_backups_days integer NOT NULL DEFAULT 7
        CHECK (deleted_backups_days BETWEEN 0 AND 90);

-- Durée envoyée au nœud à la demande de suppression, reprise à sa confirmation : un changement
-- du réglage entre les deux ne fait pas diverger le nœud et la base.
ALTER TABLE servers ADD COLUMN keep_backups_days integer;

-- Les lignes de `backups` partent avec le serveur (cascade) : copie de celles qui étaient prêtes,
-- avec ce qu'il faut pour reconnaître le serveur disparu.
CREATE TABLE deleted_server_backups (
    id                uuid        PRIMARY KEY,
    server_id         uuid        NOT NULL,
    server_name       text        NOT NULL,
    prefab_id         text        NOT NULL,
    prefab_version    text        NOT NULL,
    owner_ref         text,
    node_id           uuid        NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    name              text        NOT NULL,
    size_bytes        bigint,
    sha256            text,
    created_at        timestamptz NOT NULL,
    deleted_at        timestamptz NOT NULL DEFAULT now(),
    purge_at          timestamptz NOT NULL
);

CREATE INDEX deleted_server_backups_server ON deleted_server_backups (server_id);
CREATE INDEX deleted_server_backups_purge ON deleted_server_backups (purge_at);
