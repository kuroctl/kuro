-- Contenu Steam Workshop (ADR-0030) : objets gardés par leur identifiant, en mode `reference`.
ALTER TABLE server_content DROP CONSTRAINT server_content_source_check;
ALTER TABLE server_content ADD CONSTRAINT server_content_source_check
    CHECK (source IN ('modrinth', 'curseforge', 'steam-workshop', 'upload', 'modpack'));
