-- Mise à jour planifiée (ADR-0024, amendement) : une étape `upgrade` passe le serveur à une
-- version plus récente de son prefab (payload : la version cible, NULL pour la dernière publiée).
ALTER TABLE server_schedule_steps DROP CONSTRAINT server_schedule_steps_action_check;
ALTER TABLE server_schedule_steps
    ADD CONSTRAINT server_schedule_steps_action_check
    CHECK (action IN ('command', 'power', 'backup', 'action', 'upgrade'));
