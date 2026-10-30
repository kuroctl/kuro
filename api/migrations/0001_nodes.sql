-- Nœuds (hôtes exécutant un daemon). Sous-ensemble de l'ADR-0002.
CREATE TABLE nodes (
    id                uuid        PRIMARY KEY,
    name              text        NOT NULL UNIQUE,
    status            text        NOT NULL CHECK (status IN ('pending', 'online', 'offline')),
    -- SHA-256 hexadécimal du certificat client ; NULL tant que le nœud n'est pas enrôlé.
    cert_fingerprint  text        UNIQUE,
    daemon_version    text,
    os                text,
    arch              text,
    cpu_cores         integer,
    memory_bytes      bigint,
    last_heartbeat_at timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);
