-- Vignette portrait d'un prefab (jaquette des cartes du catalogue), en plus de l'icône et de la
-- bannière. Cosmétique comme elles : ajoutée à l'import si absente, jamais remplacée.
ALTER TABLE prefab_assets
    DROP CONSTRAINT prefab_assets_kind_check,
    ADD CONSTRAINT prefab_assets_kind_check CHECK (kind IN ('icon', 'cover', 'thumbnail'));
