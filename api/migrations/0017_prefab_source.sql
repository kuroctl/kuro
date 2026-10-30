-- Origine d'un prefab (ADR-0011) : officiel (dossier `prefabs/` importé au démarrage) ou importé
-- depuis le panel (archive `.kprefab`), et l'admin qui l'a importé.
ALTER TABLE prefabs
    ADD COLUMN source      text NOT NULL DEFAULT 'official' CHECK (source IN ('official', 'uploaded')),
    ADD COLUMN imported_by uuid REFERENCES admin_users (id) ON DELETE SET NULL;
