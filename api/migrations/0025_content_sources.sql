-- Sources de contenu à clé d'API (CurseForge) : la clé de l'installation, chiffrée avec la clé
-- des secrets des serveurs, jamais renvoyée.
CREATE TABLE content_sources (
    provider   text        PRIMARY KEY CHECK (provider IN ('curseforge')),
    api_key    bytea       NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Contenu CurseForge : fichiers vérifiés par SHA-1, la seule empreinte que la source publie.
-- Mode `reference` (ARK) : le jeu télécharge lui-même, Kuro ne garde que l'identifiant.
ALTER TABLE server_content DROP CONSTRAINT server_content_source_check;
ALTER TABLE server_content ADD CONSTRAINT server_content_source_check
    CHECK (source IN ('modrinth', 'curseforge', 'upload', 'modpack'));
ALTER TABLE server_content DROP CONSTRAINT server_content_kind_check;
ALTER TABLE server_content ADD CONSTRAINT server_content_kind_check
    CHECK (kind IN ('file', 'modpack', 'reference'));
ALTER TABLE server_content ADD COLUMN sha1 text;
