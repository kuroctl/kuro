-- Adresse vue par l'API sur le lien du nœud (pair TCP, ADR-0047) : repli pour joindre le nœud
-- quand il n'a ni nom DNS ni adresse publique saisie. Mise à jour à chaque `hello`.
ALTER TABLE nodes ADD COLUMN link_address text;
