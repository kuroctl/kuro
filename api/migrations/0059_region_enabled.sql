-- Région activée ou désactivée (ADR-0053). Une région désactivée n'est plus proposée aux
-- extensions qui vendent des serveurs ; ses nœuds et serveurs existants ne changent pas.
ALTER TABLE regions ADD COLUMN enabled boolean NOT NULL DEFAULT true;
