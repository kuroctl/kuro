-- Compte Steam d'installation (ADR-0033) : pour les serveurs que SteamCMD refuse en anonyme
-- (Arma 3, DayZ). Mot de passe chiffré avec la clé des secrets des serveurs, jamais renvoyé.
-- `scope` : `installation` (un seul compte) ; un compte apporté par un client viendra plus tard.
CREATE TABLE steam_accounts (
    id          uuid        PRIMARY KEY,
    scope       text        NOT NULL CHECK (scope IN ('installation')),
    username    text        NOT NULL,
    password    bytea       NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX steam_accounts_installation ON steam_accounts (scope)
    WHERE scope = 'installation';
