-- Notifications et webhooks (ADR-0044) : canaux (webhook générique, Discord, Slack), règles
-- (quels événements, quelle portée, vers quels canaux) et journal des envois.

CREATE TABLE notification_channels (
    id          uuid        PRIMARY KEY,
    name        text        NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    kind        text        NOT NULL CHECK (kind IN ('webhook', 'discord', 'slack')),
    -- Configuration chiffrée (URL du webhook, secret HMAC) : l'URL porte souvent un jeton.
    config      bytea       NOT NULL,
    -- URL tronquée (hôte et début du chemin), seule forme rendue par l'API.
    url_hint    text        NOT NULL,
    language    text        NOT NULL DEFAULT 'fr' CHECK (language IN ('fr', 'en')),
    enabled     boolean     NOT NULL DEFAULT true,
    created_by  uuid        REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE notification_rules (
    id          uuid        PRIMARY KEY,
    name        text        NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    scope       text        NOT NULL CHECK (scope IN ('all', 'node', 'server')),
    node_id     uuid        REFERENCES nodes (id) ON DELETE CASCADE,
    server_id   uuid        REFERENCES servers (id) ON DELETE CASCADE,
    -- Types d'événements (`server.crashed`…), au moins un.
    events      text[]      NOT NULL CHECK (cardinality(events) >= 1),
    -- Canaux destinataires, au moins un ; un canal supprimé est retiré des règles par l'API.
    channels    uuid[]      NOT NULL CHECK (cardinality(channels) >= 1),
    enabled     boolean     NOT NULL DEFAULT true,
    created_by  uuid        REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CHECK ((scope = 'node') = (node_id IS NOT NULL)),
    CHECK ((scope = 'server') = (server_id IS NOT NULL))
);

-- Un envoi par (règle qui correspond, canal) : envoyé par le worker, repris avec attente
-- croissante, abandonné après quatre tentatives. Gardé trente jours.
CREATE TABLE notification_deliveries (
    id              uuid        PRIMARY KEY,
    rule_id         uuid        REFERENCES notification_rules (id) ON DELETE SET NULL,
    channel_id      uuid        NOT NULL REFERENCES notification_channels (id) ON DELETE CASCADE,
    event           jsonb       NOT NULL,
    status          text        NOT NULL CHECK (status IN ('pending', 'sent', 'failed', 'dropped')),
    attempts        integer     NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL DEFAULT now(),
    last_error      text,
    created_at      timestamptz NOT NULL DEFAULT now(),
    sent_at         timestamptz
);

CREATE INDEX notification_deliveries_due_idx ON notification_deliveries (next_attempt_at)
    WHERE status = 'pending';
CREATE INDEX notification_deliveries_created_idx ON notification_deliveries (created_at);
CREATE INDEX notification_deliveries_channel_sent_idx ON notification_deliveries (channel_id, sent_at)
    WHERE status = 'sent';
