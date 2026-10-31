-- Certificats des nœuds supprimés (ADR-0046) : un daemon qui se présente encore avec l'un d'eux
-- est refusé comme révoqué (il s'arrête pour de bon), pas comme inconnu (il réessaierait sans fin).
CREATE TABLE revoked_certificates (
    fingerprint text        PRIMARY KEY,
    node_name   text        NOT NULL,
    revoked_at  timestamptz NOT NULL DEFAULT now()
);
