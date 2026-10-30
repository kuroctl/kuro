-- Tokens d'enrôlement à usage unique. Seul leur hash SHA-256 est stocké.
CREATE TABLE enrollment_tokens (
    id         uuid        PRIMARY KEY,
    node_id    uuid        NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    token_hash text        NOT NULL UNIQUE,
    expires_at timestamptz NOT NULL,
    used_at    timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX enrollment_tokens_node_id_idx ON enrollment_tokens (node_id);
