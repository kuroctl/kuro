-- Élévation d'une session admin (ADR-0050) : jusqu'à quand les actions protégées (suppression
-- d'un serveur, d'un nœud, secrets des réglages…) passent sans redemander le second facteur, ou
-- le mot de passe à un compte sans TOTP. Une ligne par session (famille de jetons de
-- rafraîchissement), remplacée à chaque élévation.
CREATE TABLE admin_session_elevations (
    family_id      uuid        PRIMARY KEY,
    user_id        uuid        NOT NULL REFERENCES admin_users (id) ON DELETE CASCADE,
    elevated_until timestamptz NOT NULL
);
