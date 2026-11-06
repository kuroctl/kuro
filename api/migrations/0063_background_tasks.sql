-- Tâches de fond de Kuro et des extensions (ADR-0062) : réglage de simulation et dernier passage.
-- Une ligne par tâche déclarée, créée au démarrage.
CREATE TABLE background_tasks (
    name          text PRIMARY KEY,
    dry_run       boolean     NOT NULL,
    started_at    timestamptz,
    finished_at   timestamptz,
    duration_ms   bigint,
    outcome       text CHECK (outcome IN ('ok', 'error')),
    error         text,
    report        jsonb,
    last_dry_run  boolean
);
