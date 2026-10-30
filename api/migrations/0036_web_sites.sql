-- Sites web (ADR-0038) : domaine des sites dans la zone DNS, identifiant de site par serveur.

-- Domaine sous lequel chaque site est publié (`web.kuroctl.com`) ; absent : aucun prefab web
-- n'est déployable.
ALTER TABLE dns_zones ADD COLUMN sites_domain text;

-- Identifiant aléatoire du site (10 caractères base32), attribué à la création d'un serveur dont
-- le prefab déclare un port web : `<web_id>.<sites_domain>`.
ALTER TABLE servers ADD COLUMN web_id text;
CREATE UNIQUE INDEX servers_web_id_idx ON servers (web_id) WHERE web_id IS NOT NULL;
