-- Un nom DNS par serveur et par nœud (ADR-0026).

-- Zone gérée par Kuro : domaine de base des serveurs (`play.kuroctl.com`), sous-domaine des
-- nœuds, zone et jeton chez le fournisseur. Le jeton est chiffré comme les secrets des
-- serveurs ; il n'est jamais renvoyé.
CREATE TABLE dns_zones (
    id            uuid        PRIMARY KEY,
    provider      text        NOT NULL,
    base_domain   text        NOT NULL UNIQUE,
    nodes_label   text        NOT NULL DEFAULT 'nodes',
    provider_zone text        NOT NULL,
    token         bytea       NOT NULL,
    ttl           integer     NOT NULL DEFAULT 300 CHECK (ttl BETWEEN 60 AND 86400),
    enabled       boolean     NOT NULL DEFAULT true,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

-- Adresse publique d'un nœud, saisie par l'admin (derrière un NAT, l'API ne la voit pas).
ALTER TABLE nodes
    ADD COLUMN public_ipv4 text,
    ADD COLUMN public_ipv6 text;

-- Nom d'un serveur dans la zone (`survie` pour `survie.play.kuroctl.com`).
ALTER TABLE servers ADD COLUMN dns_label text;
CREATE UNIQUE INDEX servers_dns_label_idx ON servers (dns_label) WHERE dns_label IS NOT NULL;

-- Enregistrements créés par Kuro, et eux seuls : Kuro ne touche jamais un enregistrement qu'il
-- n'a pas créé. Pas de clé étrangère vers le propriétaire : un enregistrement survit à son
-- nœud ou à son serveur le temps d'être effacé chez le fournisseur.
CREATE TABLE dns_records (
    id          uuid        PRIMARY KEY,
    zone_id     uuid        NOT NULL REFERENCES dns_zones (id) ON DELETE CASCADE,
    owner_kind  text        NOT NULL CHECK (owner_kind IN ('node', 'server')),
    owner_id    uuid        NOT NULL,
    kind        text        NOT NULL CHECK (kind IN ('A', 'AAAA', 'CNAME', 'SRV')),
    name        text        NOT NULL,
    -- Valeur publiée ; SRV : `priorité poids port cible`.
    content     text        NOT NULL,
    -- Identifiant chez le fournisseur ; absent tant que l'enregistrement n'y est pas créé.
    provider_id text,
    synced_at   timestamptz,
    error       text,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (zone_id, kind, name)
);

CREATE INDEX dns_records_owner_idx ON dns_records (owner_kind, owner_id);
