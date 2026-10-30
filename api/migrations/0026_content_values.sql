-- Valeurs saisies pour chaque contenu (ADR-0030) : champ du prefab → liste de valeurs, offertes
-- aux gabarits par `content.<emplacement>.values.<champ>`.
ALTER TABLE server_content
    ADD COLUMN field_values jsonb NOT NULL DEFAULT '{}'::jsonb;
