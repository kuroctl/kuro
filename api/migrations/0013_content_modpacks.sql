-- Modpack installé (ADR-0020) : une ligne sans fichier propre, dont dépendent ses fichiers.
ALTER TABLE server_content
    ADD COLUMN kind text NOT NULL DEFAULT 'file' CHECK (kind IN ('file', 'modpack'));
