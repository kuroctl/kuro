-- Sites web (ADR-0038) : état du proxy du nœud et du certificat d'un serveur web, rapporté par
-- le daemon avec `server.state` (`{"proxy": "ready", "certificate": "pending", "message": …}`).
ALTER TABLE servers ADD COLUMN web_status jsonb;
