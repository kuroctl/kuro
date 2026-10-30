-- Régions (emplacement physique des nœuds), placement et libellé des nœuds, couleur des rôles.
-- La région est un fait sur la machine, pas une hiérarchie : elle reste facultative.

CREATE TABLE regions (
    id         uuid             PRIMARY KEY,
    slug       text             NOT NULL UNIQUE,
    name       text             NOT NULL,
    -- Centre de la région sur la carte (position par défaut de ses nœuds).
    latitude   double precision NOT NULL CHECK (latitude BETWEEN -90 AND 90),
    longitude  double precision NOT NULL CHECK (longitude BETWEEN -180 AND 180),
    created_at timestamptz      NOT NULL DEFAULT now(),
    updated_at timestamptz      NOT NULL DEFAULT now()
);

ALTER TABLE nodes
    -- RESTRICT : une région contenant des nœuds ne peut pas être supprimée.
    ADD COLUMN region_id    uuid REFERENCES regions (id) ON DELETE RESTRICT,
    -- Libellé modifiable ; `name` reste l'identifiant technique inscrit dans le certificat.
    ADD COLUMN display_name text,
    ADD COLUMN location     text,
    ADD COLUMN latitude     double precision CHECK (latitude BETWEEN -90 AND 90),
    ADD COLUMN longitude    double precision CHECK (longitude BETWEEN -180 AND 180),
    ADD CONSTRAINT nodes_coordinates_together CHECK ((latitude IS NULL) = (longitude IS NULL));

CREATE INDEX nodes_region_id_idx ON nodes (region_id);

ALTER TABLE roles
    ADD COLUMN color text NOT NULL DEFAULT '#64748b' CHECK (color ~ '^#[0-9a-f]{6}$');
