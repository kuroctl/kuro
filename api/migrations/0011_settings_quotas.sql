-- Paramètres du projet, quota disque et suspension (ADR-0019).

-- Une seule ligne : valeurs par défaut et plafonds des ressources, conservation de l'audit.
-- Un plafond NULL : aucun. Disque 0 : illimité.
CREATE TABLE platform_settings (
    singleton            boolean     PRIMARY KEY DEFAULT true CHECK (singleton),
    default_memory_mb    integer     NOT NULL DEFAULT 2048 CHECK (default_memory_mb BETWEEN 64 AND 1048576),
    max_memory_mb        integer     CHECK (max_memory_mb BETWEEN 64 AND 1048576),
    default_cpu_percent  integer     NOT NULL DEFAULT 200 CHECK (default_cpu_percent BETWEEN 1 AND 12800),
    max_cpu_percent      integer     CHECK (max_cpu_percent BETWEEN 1 AND 12800),
    default_disk_mb      integer     NOT NULL DEFAULT 10240 CHECK (default_disk_mb BETWEEN 0 AND 104857600),
    max_disk_mb          integer     CHECK (max_disk_mb BETWEEN 1 AND 104857600),
    default_backup_limit integer     NOT NULL DEFAULT 3 CHECK (default_backup_limit BETWEEN 0 AND 100),
    max_backup_limit     integer     CHECK (max_backup_limit BETWEEN 0 AND 100),
    audit_retention_days integer     NOT NULL DEFAULT 365 CHECK (audit_retention_days BETWEEN 30 AND 3650),
    updated_at           timestamptz NOT NULL DEFAULT now(),
    CHECK (max_memory_mb IS NULL OR default_memory_mb <= max_memory_mb),
    CHECK (max_cpu_percent IS NULL OR default_cpu_percent <= max_cpu_percent),
    CHECK (max_disk_mb IS NULL OR default_disk_mb BETWEEN 1 AND max_disk_mb),
    CHECK (max_backup_limit IS NULL OR default_backup_limit <= max_backup_limit)
);
INSERT INTO platform_settings DEFAULT VALUES;

-- Suspension par un admin, et espace disque mesuré par le daemon (volumes et sauvegardes).
ALTER TABLE servers
    ADD COLUMN suspended_at     timestamptz,
    ADD COLUMN disk_used_bytes  bigint CHECK (disk_used_bytes >= 0),
    ADD COLUMN disk_measured_at timestamptz;
