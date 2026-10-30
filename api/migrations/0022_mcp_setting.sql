-- Serveur MCP (ADR-0023) : activé ou coupé à chaud depuis les paramètres, coupé par défaut.
ALTER TABLE platform_settings ADD COLUMN mcp_enabled boolean NOT NULL DEFAULT false;
