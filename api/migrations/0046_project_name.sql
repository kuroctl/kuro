-- Première mise en route (ADR-0045) : nom du projet, affiché sur la page de connexion, dans le
-- panel et les messages. NULL : « Kuro ».
ALTER TABLE platform_settings
    ADD COLUMN project_name text CHECK (length(project_name) BETWEEN 1 AND 80);
