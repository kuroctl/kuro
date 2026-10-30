-- Clé de l'API Web de Steam (Steam Workshop, ADR-0030), réglée dans le panel comme celle de
-- CurseForge.
ALTER TABLE content_sources DROP CONSTRAINT content_sources_provider_check;
ALTER TABLE content_sources ADD CONSTRAINT content_sources_provider_check
    CHECK (provider IN ('curseforge', 'steam-workshop'));
