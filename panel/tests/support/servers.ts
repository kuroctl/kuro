// Prefab, nœud et serveur simulés au format de l'API, et leurs handlers MSW.
import { http, HttpResponse } from 'msw'

import type { Server } from '@/features/servers/servers.api'
import type { Schemas } from '@/shared/api/types'

import { API } from './api'

export const NODE_ID = '01a17a00-0000-7000-8000-000000000001'
export const SERVER_ID = '01a17a00-0000-7000-8000-0000000000aa'

export const node: Schemas['NodeDto'] = {
  id: NODE_ID,
  name: 'dev-1',
  display_name: 'Dev 1',
  status: 'online',
  created_at: '2026-10-01T00:00:00Z',
  trials: false,
}

/** Sous-ensemble fidèle du manifest `minecraft-java` (ordre du fichier conservé). */
export const manifest = {
  schema: 'kuro.prefab/v1',
  id: 'minecraft-java',
  version: '1.0.0',
  category: 'game',
  services: {
    server: {
      image: 'docker.io/itzg/minecraft-server:2026.10.1-java25',
      ports: { game: { container: 25565, public: true } },
      volumes: { data: { path: '/data' } },
    },
  },
  files: {
    'server-properties': { service: 'server', path: '/data/server.properties', format: 'properties', mode: 'managed-keys' },
  },
  settings: {
    general: {
      server: {
        icon: 'server',
        fields: {
          motd: { type: 'string', default: 'A Kuro server', max_length: 59 },
          max_players: { type: 'int', default: 20, min: 1, max: 500, restart: true },
          difficulty: { type: 'enum', options: ['peaceful', 'easy', 'normal', 'hard'], default: 'normal' },
        },
      },
      version: {
        icon: 'package',
        fields: {
          loader: { type: 'enum', options: ['vanilla', 'fabric'], default: 'vanilla', editable: 'deploy', restart: true },
        },
      },
    },
    advanced: {
      security: { icon: 'shield', fields: { rcon_password: { type: 'secret', generate: { length: 32 } } } },
    },
  },
  lifecycle: { service: 'server', stop: { console: 'stop' }, timeout: 60 },
}

export const prefabSummary: Schemas['PrefabSummaryDto'] = {
  id: 'minecraft-java',
  version: '1.0.0',
  category: 'game',
  tags: ['minecraft'],
  names: { fr: 'Minecraft Java Edition', en: 'Minecraft Java Edition' },
  descriptions: { fr: 'Serveur Minecraft Java.', en: 'Minecraft Java server.' },
  deployable: true,
  resources: {},
  over_ceiling: [],
  draft_reason: null,
  steam_account_missing: false,
  test_nodes_only: false,
  assets: [],
  source: 'official',
  servers: 0,
  imported_at: '2026-10-01T00:00:00Z',
}

export const localeFr = {
  prefab: { name: 'Minecraft Java Edition', description: 'Serveur Minecraft Java.' },
  groups: { server: { title: 'Serveur' }, version: { title: 'Version et loader' }, security: { title: 'Sécurité' } },
  fields: {
    motd: { label: "Message d'accueil", help: 'Affiché dans la liste des serveurs.' },
    max_players: { label: 'Joueurs maximum' },
    difficulty: { label: 'Difficulté', options: { peaceful: 'Paisible', easy: 'Facile', normal: 'Normale', hard: 'Difficile' } },
    loader: { label: 'Loader', options: { vanilla: 'Vanilla', fabric: 'Fabric' } },
    rcon_password: { label: 'Mot de passe RCON' },
  },
  files: { 'server-properties': { description: 'Configuration du serveur.' } },
}

export function serverDto(overrides: Partial<Server> = {}): Server {
  return {
    id: SERVER_ID,
    name: 'Survie',
    node: { id: NODE_ID, name: 'dev-1' },
    prefab: { id: 'minecraft-java', version: '1.0.0' },
    memory_mb: 2048,
    cpu_percent: 200,
    disk_mb: 10240,
    backup_limit: 3,
    state: 'offline',
    state_reason: null,
    suspended_at: null,
    disk_used_bytes: null,
    disk_measured_at: null,
    settings: { motd: 'A Kuro server', max_players: 20, difficulty: 'normal', loader: 'fabric' },
    secrets: ['rcon_password'],
    allocations: [{ name: 'game', ip: '0.0.0.0', port: 25565 }],
    created_at: '2026-10-26T10:00:00Z',
    updated_at: '2026-10-26T10:00:00Z',
    restart_required: false,
    pending_changes: { contents: 0, removals: 0 },
    ...overrides,
  }
}

/** Catalogue, prefab, traductions et nœuds ; les serveurs sont fournis par chaque test. */
export const catalog = () => [
  http.get(`${API}/prefabs`, () => HttpResponse.json([prefabSummary])),
  http.get(`${API}/prefabs/minecraft-java/1.0.0`, () =>
    HttpResponse.json({ ...prefabSummary, manifest, locales: ['en', 'fr'] }),
  ),
  http.get(`${API}/prefabs/minecraft-java/1.0.0/locales/fr`, () => HttpResponse.json(localeFr)),
  http.get(`${API}/prefabs/minecraft-java/1.0.0/locales/en`, () => HttpResponse.json({})),
  http.get(`${API}/nodes`, () => HttpResponse.json([node])),
  // Mises à jour de prefab (ADR-0024) : aucune par défaut.
  http.get(`${API}/servers/:id/upgrades`, () => HttpResponse.json({ current: '1.0.0', available: [] })),
  http.get(`${API}/prefabs/:id/:version/upgrades`, () => HttpResponse.json([])),
]
