-- Déploiement d'essai d'un brouillon du Studio (ADR-0011) : le brouillon est figé en une
-- version d'essai cachée (`source = 'trial'`, version `x.y.z-trial.<n>`), déployée sur un nœud
-- de test en serveur à durée limitée, supprimé à son expiration.

-- Nœud qui accepte les essais du Studio (réglé par l'admin).
ALTER TABLE nodes ADD COLUMN trials boolean NOT NULL DEFAULT false;

ALTER TABLE prefabs
    DROP CONSTRAINT prefabs_source_check,
    ADD CONSTRAINT prefabs_source_check CHECK (source IN ('official', 'uploaded', 'trial')),
    -- Brouillon figé par une version d'essai.
    ADD COLUMN draft_id uuid REFERENCES prefab_drafts (id) ON DELETE SET NULL;

ALTER TABLE servers
    -- Serveur d'essai : brouillon essayé et fin de l'essai.
    ADD COLUMN trial_draft_id uuid REFERENCES prefab_drafts (id) ON DELETE SET NULL,
    ADD COLUMN expires_at     timestamptz;

CREATE INDEX servers_expires_at_idx ON servers (expires_at) WHERE expires_at IS NOT NULL;
