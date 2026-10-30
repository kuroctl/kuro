-- Changements de contenu en attente du prochain démarrage : un serveur en marche n'est plus
-- redémarré quand on ajoute, retire ou désactive un contenu. Le panel le signale (serveur à
-- redémarrer, contenus concernés, retraits comptés) ; tout est effacé quand le serveur démarre.
ALTER TABLE servers ADD COLUMN restart_required boolean NOT NULL DEFAULT false;
ALTER TABLE servers ADD COLUMN pending_removals integer NOT NULL DEFAULT 0
    CHECK (pending_removals >= 0);
ALTER TABLE server_content ADD COLUMN pending boolean NOT NULL DEFAULT false;
