-- Journal d'audit (ADR-0008) : append-only, chaîné par hash, sans clé étrangère
-- (il survit à la suppression des comptes et des ressources qu'il mentionne).
CREATE TABLE audit_log (
    seq         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id          uuid        NOT NULL UNIQUE,
    occurred_at timestamptz NOT NULL,
    actor_kind  text        NOT NULL
        CHECK (actor_kind IN ('admin', 'customer', 'node', 'cli', 'system', 'anonymous')),
    actor_id    uuid,
    action      text        NOT NULL,
    outcome     text        NOT NULL CHECK (outcome IN ('success', 'denied', 'failure')),
    target_kind text,
    target_id   text,
    ip          text,
    user_agent  text,
    request_id  text,
    details     jsonb       NOT NULL DEFAULT '{}',
    -- SHA-256 de l'événement précédent et de celui-ci (voir `audit::domain::chain`).
    prev_hash   text        NOT NULL,
    hash        text        NOT NULL
);

CREATE INDEX audit_log_occurred_at_idx ON audit_log (occurred_at);
CREATE INDEX audit_log_actor_idx ON audit_log (actor_kind, actor_id);

-- Append-only : seule la purge de rétention, activée explicitement dans sa transaction
-- (`SET LOCAL kuro.audit_purge = 'on'`), peut supprimer des lignes.
CREATE FUNCTION audit_log_append_only() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND current_setting('kuro.audit_purge', true) = 'on' THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit_log is append-only (% refused)', TG_OP;
END;
$$;

CREATE TRIGGER audit_log_no_update_delete
    BEFORE UPDATE OR DELETE ON audit_log
    FOR EACH ROW EXECUTE FUNCTION audit_log_append_only();

CREATE TRIGGER audit_log_no_truncate
    BEFORE TRUNCATE ON audit_log
    FOR EACH STATEMENT EXECUTE FUNCTION audit_log_append_only();
