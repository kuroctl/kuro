-- Installation d'une machine louée (ADR-0065) : dernière nouvelle de chaque étape, réussie
-- (`error` vide) ou en échec (`error` : code HTTP, sortie courte, `no_news`).
CREATE TABLE machine_progress (
    machine_id  uuid NOT NULL REFERENCES machines (id) ON DELETE CASCADE,
    step        text NOT NULL CHECK (step IN
        ('vm_requested', 'vm_started', 'install_started', 'enrolled', 'installed', 'online')),
    at          timestamptz NOT NULL DEFAULT now(),
    error       text,
    PRIMARY KEY (machine_id, step)
);
