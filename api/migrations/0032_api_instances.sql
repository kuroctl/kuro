-- Instances de l'API (ADR-0037) : chaque `kuro-api serve` s'inscrit au démarrage et bat la
-- seconde ; une instance sans battement est morte, ses nœuds repassent `offline`.
CREATE TABLE api_instances (
    id           uuid        PRIMARY KEY,
    started_at   timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now()
);

-- Instance qui tient la session du nœud ; NULL hors session. SET NULL : supprimer une instance
-- morte libère ses nœuds, que le balayage passe `offline`.
ALTER TABLE nodes
    ADD COLUMN instance_id uuid REFERENCES api_instances (id) ON DELETE SET NULL;

CREATE INDEX nodes_instance_id_idx ON nodes (instance_id) WHERE instance_id IS NOT NULL;
