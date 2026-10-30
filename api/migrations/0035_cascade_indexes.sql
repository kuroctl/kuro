-- Index des clés étrangères parcourues par les suppressions en cascade et les vérifications
-- `RESTRICT` : sans eux, supprimer un prefab ou un contenu parcourt les tables entières.
CREATE INDEX servers_prefab_idx ON servers (prefab_id, prefab_version);
CREATE INDEX server_content_modpack_idx ON server_content (modpack_id) WHERE modpack_id IS NOT NULL;
CREATE INDEX server_content_replaces_idx ON server_content (replaces) WHERE replaces IS NOT NULL;
CREATE INDEX server_content_dependencies_required_by_idx ON server_content_dependencies (required_by);
