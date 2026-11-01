// API simulée (MSW) : réponses au format du backend, typées par les schémas OpenAPI générés.
import { http, HttpResponse } from 'msw'
import { setupServer } from 'msw/node'

import type { ErrorCode, FieldProblem, Schemas } from '@/shared/api/types'

export const API = '/api/admin/v1'
export const REQUEST_ID = '0123456789abcdef'

/** Mise en route faite (ADR-0045) : l'état par défaut de tous les tests, un test peut le remplacer. */
export const setupDone = () => http.get('/api/setup', () => HttpResponse.json({ needed: false, project_name: null }))

export const server = setupServer(setupDone())

export const profile: Schemas['ProfileDto'] = {
  user: {
    id: '01a1786d-d121-71d1-a067-40612b7a2c50',
    email: 'admin@example.com',
    display_name: 'Admin',
    totp_enabled: false,
    disabled: false,
    password_change_required: false,
    invitation_pending: false,
    created_at: '2026-10-01T00:00:00Z',
    roles: [{ id: '01a1786d-d121-71d1-a067-40612b7a2c51', name: 'super-admin' }],
  },
  permissions: [],
  all_permissions: true,
  totp_required: false,
  recovery_codes_remaining: 0,
}

export function accessToken(token = 'token-1'): Schemas['AccessTokenResponse'] {
  return { access_token: token, token_type: 'Bearer', expires_in: 900 }
}

/** Réponse d'erreur RFC 9457, avec l'en-tête `x-request-id` posé par le backend. */
export function problem(
  status: number,
  code: ErrorCode | string,
  extra: { params?: Record<string, unknown>; fields?: FieldProblem[] } = {},
) {
  return HttpResponse.json(
    { type: 'about:blank', status, title: code, code, ...extra },
    { status, headers: { 'content-type': 'application/problem+json', 'x-request-id': REQUEST_ID } },
  )
}

/** Visiteur sans cookie de rafraîchissement. */
export const anonymous = () => [http.post(`${API}/auth/refresh`, () => problem(401, 'unauthorized'))]

/** Session reprise depuis le cookie, profil disponible. */
export const signedIn = () => [
  http.post(`${API}/auth/refresh`, () => HttpResponse.json(accessToken())),
  http.get(`${API}/me`, () => HttpResponse.json(profile)),
  serverSearch(),
  // Zone DNS publique (ADR-0026) : non configurée par défaut, les adresses restent `IP:port`.
  http.get(`${API}/dns`, () => HttpResponse.json({ base_domain: null, nodes_label: null })),
  // SMTP (ADR-0046) : non réglé par défaut, les comptes se créent avec un mot de passe temporaire.
  http.get(`${API}/settings/mail`, () => HttpResponse.json(mailNotConfigured)),
  // SFTP (ADR-0047) : aucune clé, et le SFTP coupé dans le projet.
  http.get(`${API}/me/ssh-keys`, () => HttpResponse.json([])),
  http.get(`${API}/servers/:id/sftp`, () => HttpResponse.json(sftpDisabled)),
  // Historique des fichiers (ADR-0049) : aucun fichier supprimé à rendre.
  http.get(`${API}/servers/:id/files/deleted`, ({ request }) =>
    HttpResponse.json({ path: new URL(request.url).searchParams.get('path'), files: [] }),
  ),
]

export const sftpDisabled: Schemas['SftpConnectionDto'] = {
  enabled: false,
  available: true,
  host: 'node-1.nodes.play.example.com',
  port: 2022,
  username: 'a1b2c3d4e5.f6a7b8c9d0',
  host_key_fingerprint: 'SHA256:3q2+7wAbCdEfGhIjKlMnOpQrStUvWxYz0123456789abc',
  node_name: 'node-1',
}

/** Réglages SMTP absents (ADR-0046). */
export const mailNotConfigured: Schemas['MailSettingsDto'] = {
  configured: false,
  insecure_allowed: false,
  host: null,
  port: null,
  security: null,
  username: null,
  has_password: false,
  from_address: null,
  from_name: null,
  updated_at: null,
}

type ServerDto = Schemas['ServerDto']

/**
 * Liste paginée des serveurs (ADR-0025) bâtie sur la liste simulée par le test
 * (`GET /servers`) : recherche par nom, filtres prefab et état, facettes, tri par nom.
 */
export function serverSearch() {
  return http.get(`${API}/servers/search`, async ({ request }) => {
    const url = new URL(request.url)
    const all: ServerDto[] = await (await fetch(new URL(`${API}/servers`, url))).json()
    const list = (key: string) => url.searchParams.get(key)?.split(',').filter(Boolean) ?? []
    const q = url.searchParams.get('q')?.toLowerCase()
    const prefabs = list('prefab')
    const states = list('state')
    const keep = (s: ServerDto, except?: string) =>
      (!q || s.name.toLowerCase().includes(q)) &&
      (except === 'prefab' || !prefabs.length || prefabs.includes(s.prefab.id)) &&
      (except === 'state' || !states.length || states.includes(s.state))
    const count = (values: string[]) =>
      [...new Set(values)].sort().map((value) => ({ value, label: value, count: values.filter((v) => v === value).length }))
    const items = all.filter((s) => keep(s)).sort((a, b) => a.name.localeCompare(b.name))
    const page: Schemas['ServerPageDto'] = {
      items,
      total: items.length,
      page: 1,
      per_page: 25,
      facets: {
        prefab: count(all.filter((s) => keep(s, 'prefab')).map((s) => s.prefab.id)),
        node: [],
        state: count(all.filter((s) => keep(s, 'state')).map((s) => s.state)),
        upgrade: 0,
        suspended: 0,
      },
      upgrades: {},
      ids: url.searchParams.get('all_ids') === 'true' ? items.map((s) => s.id) : null,
    }
    return HttpResponse.json(page)
  })
}

/** Sorties de l'API qu'un parcours peut déclencher sans que le test s'y intéresse. */
export const logout = () => http.post(`${API}/auth/logout`, () => new HttpResponse(null, { status: 204 }))
