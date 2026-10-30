-- Mot de passe choisi par un autre admin (création, réinitialisation depuis le panel) : à
-- remplacer à la prochaine connexion, avant toute session.
ALTER TABLE admin_users
    ADD COLUMN password_change_required boolean NOT NULL DEFAULT false;
