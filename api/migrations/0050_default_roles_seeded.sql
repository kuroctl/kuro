-- Rôles livrés (ADR-0045, amendement) : créés une seule fois, à la mise en route ou au premier
-- démarrage d'une installation antérieure. Un rôle supprimé ensuite n'est jamais recréé.
ALTER TABLE platform_settings ADD COLUMN default_roles_seeded_at timestamptz;
