-- Notifications par serveur (ADR-0044, amendement) : un canal créé depuis la page d'un serveur
-- lui appartient (il disparaît avec lui, n'apparaît pas dans les paramètres globaux). Un
-- abonnement du serveur = ce canal (ou un canal partagé) + une règle de portée serveur.
ALTER TABLE notification_channels
    ADD COLUMN server_id uuid REFERENCES servers (id) ON DELETE CASCADE;
CREATE INDEX notification_channels_server_idx ON notification_channels (server_id)
    WHERE server_id IS NOT NULL;
