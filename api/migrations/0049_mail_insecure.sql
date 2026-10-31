-- SMTP sans chiffrement (ADR-0046) : accepté par l'API seulement avec `mail.allow_insecure`
-- (boîte de dev), la contrainte doit le permettre.
ALTER TABLE mail_settings DROP CONSTRAINT mail_settings_security_check;
ALTER TABLE mail_settings
    ADD CONSTRAINT mail_settings_security_check CHECK (security IN ('starttls', 'tls', 'none'));
