-- Rétention de l'historique des fichiers (ADR-0049), appliquée par chaque nœud : durée d'une
-- version (0 : historique coupé) et place prise au plus par serveur.
ALTER TABLE platform_settings
    ADD COLUMN file_history_days   integer NOT NULL DEFAULT 30
        CHECK (file_history_days BETWEEN 0 AND 365),
    ADD COLUMN file_history_max_mb integer NOT NULL DEFAULT 100
        CHECK (file_history_max_mb BETWEEN 1 AND 10240);
