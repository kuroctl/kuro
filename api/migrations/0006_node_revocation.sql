-- Statut `revoked` (ADR-0005) : l'empreinte du certificat est conservée pour reconnaître le
-- nœud et le refuser ; un nouveau token le repasse `pending` (réenrôlement).
ALTER TABLE nodes DROP CONSTRAINT nodes_status_check;
ALTER TABLE nodes ADD CONSTRAINT nodes_status_check
    CHECK (status IN ('pending', 'online', 'offline', 'revoked'));
