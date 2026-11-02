-- Tickets de console du propriétaire d'un serveur (ADR-0054) : un ticket appartient à un admin ou
-- à un propriétaire (`owner_ref` d'une extension, ADR-0051), jamais aux deux.
ALTER TABLE tickets ALTER COLUMN admin_id DROP NOT NULL;
ALTER TABLE tickets ADD COLUMN owner_ref text;
ALTER TABLE tickets ADD CONSTRAINT tickets_holder_check
    CHECK ((admin_id IS NULL) <> (owner_ref IS NULL));
