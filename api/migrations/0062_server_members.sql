-- Membres d'un serveur (ADR-0060) : propriétaires (`owner_ref`) admis dans l'espace client d'un
-- serveur en plus du sien, ajoutés et retirés par l'extension qui gère ses clients.
CREATE TABLE server_members (
    server_id  uuid        NOT NULL REFERENCES servers (id) ON DELETE CASCADE,
    owner_ref  text        NOT NULL,
    added_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (server_id, owner_ref)
);

CREATE INDEX server_members_owner ON server_members (owner_ref);
