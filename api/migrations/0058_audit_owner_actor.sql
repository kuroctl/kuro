-- Acteur `owner` (ADR-0051) : le propriétaire d'un serveur, identifié par une extension. Son
-- identifiant opaque (`cloud:…`) n'est pas un uuid : il va dans `details.owner_ref`, couvert par
-- le chaînage, et `actor_id` reste vide.
ALTER TABLE audit_log DROP CONSTRAINT audit_log_actor_kind_check;
ALTER TABLE audit_log ADD CONSTRAINT audit_log_actor_kind_check
    CHECK (actor_kind IN ('admin', 'customer', 'node', 'cli', 'system', 'anonymous', 'agent', 'owner'));
