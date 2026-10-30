-- Images d'un prefab (icône, bannière) : cosmétiques, ajoutées à l'import si absentes, jamais
-- remplacées. Supprimées avec leur prefab.
CREATE TABLE prefab_assets (
    prefab_id      text  NOT NULL,
    prefab_version text  NOT NULL,
    kind           text  NOT NULL CHECK (kind IN ('icon', 'cover')),
    -- Reconnu au contenu à l'import : PNG, JPEG ou WebP, jamais SVG.
    content_type   text  NOT NULL CHECK (content_type IN ('image/png', 'image/jpeg', 'image/webp')),
    data           bytea NOT NULL CHECK (octet_length(data) <= 2097152),
    PRIMARY KEY (prefab_id, prefab_version, kind),
    FOREIGN KEY (prefab_id, prefab_version) REFERENCES prefabs (id, version) ON DELETE CASCADE
);
