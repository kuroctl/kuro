-- Démarrage une fois installé : choisi à la création, consommé par l'API quand le nœud rapporte
-- la fin du déploiement (serveur arrêté), effacé si le déploiement échoue.
ALTER TABLE servers ADD COLUMN start_when_installed boolean NOT NULL DEFAULT false;
