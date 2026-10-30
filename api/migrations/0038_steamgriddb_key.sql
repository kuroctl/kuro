-- Clé de l'API de SteamGridDB (images des prefabs dans le Studio, ADR-0040), réglée dans le panel
-- comme celles de CurseForge et du Workshop Steam.
ALTER TABLE content_sources DROP CONSTRAINT content_sources_provider_check;
ALTER TABLE content_sources ADD CONSTRAINT content_sources_provider_check
    CHECK (provider IN ('curseforge', 'steam-workshop', 'steamgriddb'));
