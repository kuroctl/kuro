-- Brouillons du Prefab Studio (ADR-0011) : les fichiers texte d'un prefab en cours d'écriture
-- (`prefab.yaml`, `locales/*.json`, gabarits) et ses images, partagés entre admins jusqu'à leur
-- publication en version importée.
CREATE TABLE prefab_drafts (
    id             uuid        PRIMARY KEY,
    -- Chemin relatif → contenu texte. Bornes (nombre, tailles) vérifiées par l'API.
    files          jsonb       NOT NULL CHECK (jsonb_typeof(files) = 'object'),
    -- Lus du manifest à chaque enregistrement, s'il se lit : libellé dans la liste.
    prefab_id      text,
    prefab_version text,
    -- Version dont le brouillon est parti, le cas échéant.
    base_id        text,
    base_version   text,
    created_by     uuid        REFERENCES admin_users (id) ON DELETE SET NULL,
    updated_by     uuid        REFERENCES admin_users (id) ON DELETE SET NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);

-- Images d'un brouillon, téléversées à part (icône, bannière, vignette).
CREATE TABLE prefab_draft_assets (
    draft_id     uuid  NOT NULL REFERENCES prefab_drafts (id) ON DELETE CASCADE,
    kind         text  NOT NULL CHECK (kind IN ('icon', 'cover', 'thumbnail')),
    content_type text  NOT NULL CHECK (content_type IN ('image/png', 'image/jpeg', 'image/webp')),
    data         bytea NOT NULL CHECK (octet_length(data) <= 2097152),
    PRIMARY KEY (draft_id, kind)
);
