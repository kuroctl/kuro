-- Installation des serveurs (ADR-0017) : nouvel état `installing`, rapporté par le daemon pendant
-- les étapes d'installation (création, réinstallation, réinitialisation).
ALTER TABLE servers DROP CONSTRAINT servers_state_check;
ALTER TABLE servers ADD CONSTRAINT servers_state_check CHECK (state IN
    ('deploying', 'installing', 'offline', 'starting', 'running', 'stopping', 'failed', 'deleting'));
