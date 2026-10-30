-- Tâches planifiées d'un serveur (ADR-0022) : déclencheur cron dans un fuseau horaire, étapes
-- exécutées dans l'ordre, historique borné des exécutions. Effacées avec le serveur.

CREATE TABLE server_schedules (
    id               uuid        PRIMARY KEY,
    server_id        uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    name             text        NOT NULL,
    -- Expression à 5 champs (minute heure jour mois jour-de-semaine).
    cron             text        NOT NULL,
    -- Fuseau IANA (`Europe/Paris`) dans lequel l'expression est lue.
    timezone         text        NOT NULL,
    enabled          boolean     NOT NULL DEFAULT true,
    -- Sautée si le serveur n'est pas en ligne au moment de l'exécution.
    only_when_online boolean     NOT NULL DEFAULT false,
    -- Prochaine échéance ; NULL tant que la tâche est désactivée.
    next_run_at      timestamptz,
    -- Exécution en cours : une tâche ne se chevauche jamais elle-même.
    running_since    timestamptz,
    created_by       uuid        REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX server_schedules_server_id_idx ON server_schedules (server_id);
CREATE INDEX server_schedules_due_idx ON server_schedules (next_run_at) WHERE enabled;

CREATE TABLE server_schedule_steps (
    schedule_id         uuid    NOT NULL REFERENCES server_schedules (id) ON DELETE CASCADE,
    position            integer NOT NULL CHECK (position >= 0),
    action              text    NOT NULL CHECK (action IN ('command', 'power', 'backup')),
    -- `command` : la ligne ; `power` : start, stop, restart ou kill ; `backup` : nom ou NULL.
    payload             text,
    -- Attente avant l'étape.
    delay_secs          integer NOT NULL DEFAULT 0 CHECK (delay_secs BETWEEN 0 AND 3600),
    continue_on_failure boolean NOT NULL DEFAULT false,
    PRIMARY KEY (schedule_id, position)
);

CREATE TABLE server_schedule_runs (
    id          uuid        PRIMARY KEY,
    schedule_id uuid        NOT NULL REFERENCES server_schedules (id) ON DELETE CASCADE,
    trigger     text        NOT NULL CHECK (trigger IN ('schedule', 'manual')),
    started_at  timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    outcome     text        NOT NULL CHECK (outcome IN ('running', 'succeeded', 'failed', 'skipped')),
    -- Étape en échec (position) et code de la cause, ou cause du saut.
    failed_step integer,
    reason      text
);

CREATE INDEX server_schedule_runs_schedule_idx ON server_schedule_runs (schedule_id, started_at DESC);
