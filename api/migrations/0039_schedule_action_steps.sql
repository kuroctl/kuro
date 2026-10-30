-- Actions d'un prefab comme étapes de tâche planifiée (ADR-0039) : la liste fermée des étapes
-- s'ouvre à `action` (payload : la clé de l'action dans le prefab).
ALTER TABLE server_schedule_steps DROP CONSTRAINT server_schedule_steps_action_check;
ALTER TABLE server_schedule_steps
    ADD CONSTRAINT server_schedule_steps_action_check
    CHECK (action IN ('command', 'power', 'backup', 'action'));
