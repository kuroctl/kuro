-- Serveur créé avant son nœud (ADR-0065) : il attend la machine louée pour lui, puis il est
-- déployé dès que le nœud se connecte. La création attendue n'est plus gardée sur la machine.
ALTER TABLE servers DROP CONSTRAINT servers_state_check;
ALTER TABLE servers ADD CONSTRAINT servers_state_check CHECK (state IN
    ('waiting_node', 'deploying', 'installing', 'restoring', 'offline', 'starting', 'running',
     'stopping', 'failed', 'deleting'));

ALTER TABLE machines ADD COLUMN server_id uuid REFERENCES servers (id) ON DELETE SET NULL;
ALTER TABLE machines DROP COLUMN pending_server;
