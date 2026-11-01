-- Extension de test (ADR-0051) : table sans schéma, créée dans le schéma de l'extension, et clé
-- étrangère vers une table de Kuro (`public` reste dans le chemin pendant la migration).
create table notes (
    id bigserial primary key,
    server_id uuid references servers (id) on delete cascade,
    body text not null
);
