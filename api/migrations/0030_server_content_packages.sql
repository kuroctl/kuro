-- Paquets extraits d'un contenu en mode `extract` (ADR-0030, add-ons Bedrock) : dossier et nom
-- choisis par le nœud, description lue dans le paquet (`manifest.json`), dans l'ordre de
-- l'archive. Ils alimentent les fichiers d'inscription ; supprimés avec leur contenu.
CREATE TABLE server_content_packages (
    content_id  uuid    NOT NULL REFERENCES server_content (id) ON DELETE CASCADE,
    position    integer NOT NULL CHECK (position >= 0),
    dir         text    NOT NULL,
    name        text    NOT NULL,
    description jsonb   NOT NULL,
    PRIMARY KEY (content_id, position)
);
