-- Contenu des serveurs : mods, plugins, modpacks (ADR-0020). Effacé avec le serveur.
CREATE TABLE server_content (
    id           uuid        PRIMARY KEY,
    server_id    uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    -- Emplacement du prefab (`mods`, `plugins`…).
    slot         text        NOT NULL,
    source       text        NOT NULL CHECK (source IN ('modrinth', 'upload', 'modpack')),
    -- Projet et version chez le provider ; absents pour un fichier envoyé.
    project_id   text,
    version_id   text,
    name         text        NOT NULL,
    version_name text,
    icon_url     text,
    file_name    text        NOT NULL,
    sha512       text,
    size_bytes   bigint      CHECK (size_bytes >= 0),
    enabled      boolean     NOT NULL DEFAULT true,
    state        text        NOT NULL CHECK (state IN ('installing', 'installed', 'failed')),
    failure      text,
    -- Modpack d'origine : ses fichiers partent avec lui.
    modpack_id   uuid        REFERENCES server_content (id) ON DELETE CASCADE,
    -- Opération du nœud en cours (`content.done` la termine).
    operation_id uuid,
    -- Mise à jour : version remplacée, retirée quand celle-ci est installée.
    replaces     uuid        REFERENCES server_content (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX server_content_server_idx ON server_content (server_id, slot);
CREATE INDEX server_content_operation_idx ON server_content (operation_id) WHERE operation_id IS NOT NULL;
CREATE INDEX server_content_project_idx ON server_content (server_id, slot, project_id);

-- Dépendance obligatoire : `content_id` est exigé par `required_by`.
CREATE TABLE server_content_dependencies (
    content_id  uuid NOT NULL REFERENCES server_content (id) ON DELETE CASCADE,
    required_by uuid NOT NULL REFERENCES server_content (id) ON DELETE CASCADE,
    PRIMARY KEY (content_id, required_by)
);
