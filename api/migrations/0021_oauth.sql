-- Serveur MCP (ADR-0023) : l'API est le serveur d'autorisation OAuth 2.1 des agents IA.
-- Un admin délègue une partie de ses droits à un client (accord) ; le client agit ensuite avec
-- des jetons opaques, dont seul le SHA-256 est stocké.

-- Acteur `agent` : une IA qui agit au nom d'un admin.
ALTER TABLE audit_log DROP CONSTRAINT audit_log_actor_kind_check;
ALTER TABLE audit_log ADD CONSTRAINT audit_log_actor_kind_check
    CHECK (actor_kind IN ('admin', 'customer', 'node', 'cli', 'system', 'anonymous', 'agent'));

-- Client OAuth : document de métadonnées lu à son URL (`client_id` = cette URL), ou client
-- enregistré dynamiquement (`client_id` émis par l'API). Toujours public (PKCE, sans secret).
CREATE TABLE oauth_clients (
    id            uuid        PRIMARY KEY,
    client_id     text        NOT NULL UNIQUE,
    registration  text        NOT NULL CHECK (registration IN ('metadata', 'dynamic')),
    -- Déclarés par le client, non vérifiés : affichés comme tels.
    name          text        NOT NULL,
    uri           text,
    redirect_uris text[]      NOT NULL,
    -- Dernière lecture du document de métadonnées (cache).
    fetched_at    timestamptz,
    created_at    timestamptz NOT NULL DEFAULT now()
);

-- Accord d'un admin à un client : les droits délégués. Un seul par admin et par client ; le
-- révoquer le supprime, avec ses jetons.
CREATE TABLE oauth_grants (
    id           uuid        PRIMARY KEY,
    user_id      uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    client_id    uuid        NOT NULL REFERENCES oauth_clients (id) ON DELETE CASCADE,
    scopes       text[]      NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    UNIQUE (user_id, client_id)
);

-- Demande d'autorisation en attente du consentement. Accordée, elle porte le code (haché, usage
-- unique) qui l'efface à son échange.
CREATE TABLE oauth_requests (
    id              uuid        PRIMARY KEY,
    client_id       uuid        NOT NULL REFERENCES oauth_clients (id) ON DELETE CASCADE,
    redirect_uri    text        NOT NULL,
    state           text,
    code_challenge  text        NOT NULL,
    scopes          text[]      NOT NULL,
    expires_at      timestamptz NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT now(),
    grant_id        uuid        REFERENCES oauth_grants (id) ON DELETE CASCADE,
    code_hash       text        UNIQUE,
    code_expires_at timestamptz
);

CREATE INDEX oauth_requests_expires_at_idx ON oauth_requests (expires_at);

-- Jetons d'un accord. Un jeton de rafraîchissement échangé reste jusqu'à son expiration
-- (`consumed_at`) : le présenter à nouveau signe un vol et révoque l'accord.
CREATE TABLE oauth_tokens (
    token_hash  text        PRIMARY KEY,
    grant_id    uuid        NOT NULL REFERENCES oauth_grants (id) ON DELETE CASCADE,
    kind        text        NOT NULL CHECK (kind IN ('access', 'refresh')),
    expires_at  timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX oauth_tokens_grant_id_idx ON oauth_tokens (grant_id);
CREATE INDEX oauth_tokens_expires_at_idx ON oauth_tokens (expires_at);
