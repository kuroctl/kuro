-- Commandes déposées pour l'instance qui tient le nœud (ADR-0037) : le message entier est ici,
-- `NOTIFY` ne sert qu'à réveiller. La réponse revient dans la même ligne.
CREATE TABLE node_commands (
    id          uuid        PRIMARY KEY,
    node_id     uuid        NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    instance_id uuid        NOT NULL REFERENCES api_instances (id) ON DELETE CASCADE,
    message     jsonb       NOT NULL,
    reply       jsonb,
    -- Échec constaté par l'instance qui tient le nœud.
    failure     text        CHECK (failure IN ('offline', 'timeout', 'closed')),
    created_at  timestamptz NOT NULL DEFAULT now(),
    -- Prise en charge par l'instance cible : une commande n'est exécutée qu'une fois, même si
    -- la notification et le balayage de secours la voient tous les deux.
    claimed_at  timestamptz,
    replied_at  timestamptz
);

CREATE INDEX node_commands_pending_idx ON node_commands (instance_id, created_at)
    WHERE replied_at IS NULL;
