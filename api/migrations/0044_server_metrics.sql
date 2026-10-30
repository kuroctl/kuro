-- Stats historisées (ADR-0043) : une ligne par serveur et par minute envoyée par le nœud
-- (`server.metrics`), gardée sept jours ; un cumul par heure, gardé quatre-vingt-dix jours.
CREATE TABLE server_metrics (
    server_id          uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    minute             timestamptz NOT NULL,
    samples            integer     NOT NULL CHECK (samples >= 0),
    cpu_millis_avg     bigint      NOT NULL,
    cpu_millis_max     bigint      NOT NULL,
    memory_bytes_avg   bigint      NOT NULL,
    memory_bytes_max   bigint      NOT NULL,
    memory_limit_bytes bigint      NOT NULL,
    network_rx_bytes   bigint      NOT NULL,
    network_tx_bytes   bigint      NOT NULL,
    PRIMARY KEY (server_id, minute)
);
CREATE INDEX server_metrics_minute_idx ON server_metrics (minute);

CREATE TABLE server_metrics_hourly (
    server_id          uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    hour               timestamptz NOT NULL,
    samples            integer     NOT NULL CHECK (samples >= 0),
    cpu_millis_avg     bigint      NOT NULL,
    cpu_millis_max     bigint      NOT NULL,
    memory_bytes_avg   bigint      NOT NULL,
    memory_bytes_max   bigint      NOT NULL,
    memory_limit_bytes bigint      NOT NULL,
    network_rx_bytes   bigint      NOT NULL,
    network_tx_bytes   bigint      NOT NULL,
    PRIMARY KEY (server_id, hour)
);
CREATE INDEX server_metrics_hourly_hour_idx ON server_metrics_hourly (hour);
