-- Seuils d'alerte sur les stats (ADR-0043, amendement) : CPU et mémoire soutenus pendant
-- `alert_minutes` minutes, réglés pour tout le projet ; état actif de chaque alerte par serveur.
ALTER TABLE platform_settings
    ADD COLUMN alert_cpu_percent    integer NOT NULL DEFAULT 90 CHECK (alert_cpu_percent BETWEEN 50 AND 100),
    ADD COLUMN alert_memory_percent integer NOT NULL DEFAULT 90 CHECK (alert_memory_percent BETWEEN 50 AND 100),
    ADD COLUMN alert_minutes        integer NOT NULL DEFAULT 5 CHECK (alert_minutes BETWEEN 1 AND 60);

-- Une ligne par alerte active : posée au franchissement (un seul envoi), retirée quand la
-- moyenne repasse sous le seuil bas. Disparaît avec le serveur.
CREATE TABLE server_alerts (
    server_id uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    kind      text        NOT NULL CHECK (kind IN ('cpu', 'memory')),
    since     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (server_id, kind)
);
