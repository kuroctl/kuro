-- Serveurs déployés à partir de prefabs (ADR-0011, ADR-0013).

-- Prefabs : id + version est immuable. Manifest, traductions et gabarits sont stockés analysés ;
-- l'archive arrivera avec l'import de prefabs par l'admin.
CREATE TABLE prefabs (
    id          text        NOT NULL,
    version     text        NOT NULL,
    -- JSON en texte : l'ordre des groupes et des champs est l'ordre d'affichage, et JSONB
    -- trierait les clés.
    manifest    text        NOT NULL,
    locales     jsonb       NOT NULL,
    templates   jsonb       NOT NULL,
    imported_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (id, version)
);

CREATE TABLE servers (
    id             uuid        PRIMARY KEY,
    name           text        NOT NULL,
    -- RESTRICT : un nœud qui héberge des serveurs ne peut pas être supprimé.
    node_id        uuid        NOT NULL REFERENCES nodes (id) ON DELETE RESTRICT,
    prefab_id      text        NOT NULL,
    prefab_version text        NOT NULL,
    -- Réglages sans les secrets ; ceux-ci sont chiffrés à part (jamais renvoyés par l'API).
    settings       jsonb       NOT NULL,
    secrets        bytea,
    memory_mb      integer     NOT NULL CHECK (memory_mb BETWEEN 64 AND 1048576),
    cpu_percent    integer     NOT NULL CHECK (cpu_percent BETWEEN 1 AND 12800),
    disk_mb        integer     NOT NULL CHECK (disk_mb BETWEEN 0 AND 104857600),
    state          text        NOT NULL CHECK (state IN
        ('deploying', 'offline', 'starting', 'running', 'stopping', 'failed', 'deleting')),
    state_reason   text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (prefab_id, prefab_version) REFERENCES prefabs (id, version) ON DELETE RESTRICT
);

CREATE INDEX servers_node_id_idx ON servers (node_id);

-- Ports d'un nœud. Une allocation libre disparaît avec son nœud ; assignée, son serveur bloque
-- la suppression du nœud. Supprimer un serveur libère d'abord ses ports (`server_id` et
-- `port_name` ensemble) : RESTRICT garantit qu'aucun chemin ne l'oublie.
CREATE TABLE allocations (
    id         uuid        PRIMARY KEY,
    node_id    uuid        NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    ip         text        NOT NULL,
    port       integer     NOT NULL CHECK (port BETWEEN 1024 AND 65535),
    server_id  uuid        REFERENCES servers (id) ON DELETE RESTRICT,
    -- Port du prefab servi par cette allocation (`game`, `http`…).
    port_name  text,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (node_id, ip, port),
    CONSTRAINT allocations_assignment CHECK ((server_id IS NULL) = (port_name IS NULL))
);

CREATE INDEX allocations_server_id_idx ON allocations (server_id);
