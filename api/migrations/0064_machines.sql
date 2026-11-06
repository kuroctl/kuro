-- Nœuds à la demande (ADR-0065) : machines louées chez un hébergeur et enrôlées comme nœuds.

-- Nœud réservé : jamais proposé d'office, seulement quand on le nomme.
ALTER TABLE nodes ADD COLUMN reserved boolean NOT NULL DEFAULT false;

-- Réglages (une ligne). `instance_tag` est posé en étiquette sur chaque machine : deux instances
-- de Kuro qui partagent un compte chez le fournisseur ne se prennent pas leurs machines.
CREATE TABLE machine_settings (
    id            boolean PRIMARY KEY DEFAULT true CHECK (id),
    instance_tag  text NOT NULL DEFAULT replace(gen_random_uuid()::text, '-', ''),
    api_url       text,
    port_range    text NOT NULL DEFAULT '25565-25600'
);
INSERT INTO machine_settings DEFAULT VALUES;

-- Identifiants d'un fournisseur, chiffrés, jamais relus par l'API.
CREATE TABLE machine_providers (
    provider    text PRIMARY KEY,
    token       bytea NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Fournisseur et emplacement de chaque région ; une région sans ligne ne loue rien.
CREATE TABLE machine_regions (
    region_id  uuid PRIMARY KEY REFERENCES regions (id) ON DELETE CASCADE,
    provider   text NOT NULL,
    location   text NOT NULL
);

-- `creating` : ligne écrite avant l'appel au fournisseur (création idempotente) ;
-- `ready` : VM en place ; `sleeping` : snapshot demandé, VM encore là ; `asleep` : VM rendue,
-- snapshot gardé ; `waking` : VM recréée depuis le snapshot ; `releasing` : rendue en cours
-- (rejouée par le ménage) ; `deleted` : rendue.
CREATE TABLE machines (
    id              uuid PRIMARY KEY,
    provider        text NOT NULL,
    reference       text,
    offer           text NOT NULL,
    location        text NOT NULL,
    region_id       uuid REFERENCES regions (id) ON DELETE SET NULL,
    -- Coût horaire HT au moment de la location, en 1/100 000 d'euro.
    hourly_cost     bigint NOT NULL,
    monthly_cost    bigint NOT NULL,
    node_id         uuid REFERENCES nodes (id) ON DELETE SET NULL,
    ipv4            text,
    snapshot        text,
    -- Appelant (extension) : texte libre, jamais interprété par Kuro.
    claim           text,
    state           text NOT NULL
        CHECK (state IN ('creating', 'ready', 'sleeping', 'asleep', 'waking', 'releasing', 'deleted')),
    -- Création de serveur en attente du nœud, chiffrée (réglages et secrets saisis).
    pending_server  bytea,
    -- Signalement déjà émis (`stalled`, `lost`) : pas d'événement répété à chaque passage.
    flag            text,
    node_online_at  timestamptz,
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    deleted_at      timestamptz
);
CREATE UNIQUE INDEX machines_node_alive ON machines (node_id) WHERE state <> 'deleted';
CREATE INDEX machines_claim ON machines (claim) WHERE claim IS NOT NULL;
CREATE INDEX machines_alive ON machines (provider) WHERE state <> 'deleted';
