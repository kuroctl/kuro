-- Propriétaire d'un serveur (ADR-0051) : identifiant opaque posé par une extension
-- (`cloud:6f1c…`), jamais interprété par Kuro : préfixe de l'extension (comme son nom), deux-points,
-- identifiant sans espace. NULL : serveur interne, géré par les admins seuls.
ALTER TABLE servers
    ADD COLUMN owner_ref text
        CHECK (owner_ref ~ '^[a-z][a-z0-9_]{0,31}:[A-Za-z0-9._~-]{1,95}$');

-- Serveurs d'un propriétaire (espace client, filtre de la liste admin).
CREATE INDEX servers_owner_ref_idx ON servers (owner_ref) WHERE owner_ref IS NOT NULL;
