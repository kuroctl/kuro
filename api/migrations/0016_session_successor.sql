-- Fenêtre de grâce de la rotation des sessions admin : le jeton qui a remplacé celui-ci, chiffré
-- avec une clé dérivée de `auth.secret_key`. Une requête concurrente qui présente encore l'ancien
-- jeton pendant la grâce reçoit ce successeur, et non un cookie déjà consommé (sinon le
-- rafraîchissement suivant passerait pour un vol). Effacé une fois la grâce passée.
ALTER TABLE admin_sessions ADD COLUMN successor_secret bytea;
