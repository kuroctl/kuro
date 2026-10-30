// Agents IA (MCP, ADR-0023) : bascule du serveur dans les paramètres, page de consentement,
// applications connectées.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Schemas } from '@/shared/api/types'
import * as navigation from '@/shared/lib/navigation'

import { API, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

const REQUEST = '01a17a00-0000-7000-8000-0000000000c1'
const GRANT = '01a17a00-0000-7000-8000-0000000000c2'

const settings: Schemas['SettingsDto'] = {
  default_memory_mb: 2048,
  max_memory_mb: null,
  default_cpu_percent: 200,
  max_cpu_percent: null,
  default_disk_mb: 10240,
  max_disk_mb: null,
  default_backup_limit: 3,
  max_backup_limit: null,
  audit_retention_days: 365,
  mcp_enabled: false,
  mcp_url: 'https://kuro.test/mcp',
  updated_at: '2026-10-27T10:00:00Z',
}

const consent: Schemas['ConsentRequestDto'] = {
  id: REQUEST,
  client: {
    name: 'Claude',
    client_id: 'https://claude.ai/oauth/mcp-oauth-client-metadata',
    registration: 'metadata',
    uri: 'https://claude.ai',
    redirect_host: 'claude.ai',
  },
  scopes: [
    { key: 'nodes.read', sensitive: false, held: false },
    { key: 'servers.read', sensitive: false, held: true },
    { key: 'servers.power', sensitive: false, held: true },
  ],
  expires_at: '2026-10-28T10:10:00Z',
  access_ttl_secs: 3600,
  refresh_ttl_secs: 2592000,
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('mcp server setting', () => {
  it('switches the MCP server on at once, without the save bar', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...settings, mcp_enabled: true })
      }),
    )
    const { user } = await renderApp('/settings')

    expect(await screen.findByText('https://kuro.test/mcp')).toBeTruthy()
    await user.click(screen.getByRole('switch', { name: 'Ouvert aux agents IA' }))

    await waitFor(() => expect(body).toEqual({ mcp_enabled: true }))
    expect(await screen.findByText('Serveur MCP ouvert.')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Enregistrer' })).toBeNull()
  })

  it('cannot be switched on when the API has no public URL', async () => {
    server.use(...signedIn(), http.get(`${API}/settings`, () => HttpResponse.json({ ...settings, mcp_url: null })))
    await renderApp('/settings')

    expect(await screen.findByText(/Serveur MCP non configuré : renseignez mcp\.public_url/)).toBeTruthy()
    expect((screen.getByRole('switch', { name: 'Ouvert aux agents IA' }) as HTMLButtonElement).disabled).toBe(true)
  })
})

describe('oauth consent', () => {
  it('shows the app, the account and the rights, then sends back only the chosen ones', async () => {
    const leave = vi.spyOn(navigation, 'leaveTo').mockImplementation(() => {})
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/oauth/requests/${REQUEST}`, () => HttpResponse.json(consent)),
      http.post(`${API}/oauth/requests/${REQUEST}/approve`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ redirect_to: 'https://claude.ai/api/mcp/auth_callback?code=c&iss=x' })
      }),
    )
    const { user } = await renderApp(`/oauth/consent?request=${REQUEST}`)

    expect(await screen.findByText('Autoriser Claude à agir sur Kuro ?')).toBeTruthy()
    expect(screen.getByText('admin@example.com')).toBeTruthy()
    expect(screen.getByText('claude.ai')).toBeTruthy()
    // Hors de la coque du panel.
    expect(screen.queryByRole('navigation')).toBeNull()

    const notHeld = screen.getByRole('switch', { name: /Lister les nœuds/ }) as HTMLButtonElement
    expect(notHeld.disabled).toBe(true)
    expect(screen.getByText('Vous ne détenez pas ce droit : il ne sera pas accordé.')).toBeTruthy()
    await user.click(screen.getByRole('switch', { name: /Démarrer, arrêter/ }))
    await user.click(screen.getByRole('button', { name: 'Autoriser' }))

    await waitFor(() => expect(body).toEqual({ scopes: ['servers.read'] }))
    expect(leave).toHaveBeenCalledWith('https://claude.ai/api/mcp/auth_callback?code=c&iss=x')
  })

  it('denies and sends the browser back to the app', async () => {
    const leave = vi.spyOn(navigation, 'leaveTo').mockImplementation(() => {})
    server.use(
      ...signedIn(),
      http.get(`${API}/oauth/requests/${REQUEST}`, () => HttpResponse.json(consent)),
      http.post(`${API}/oauth/requests/${REQUEST}/deny`, () =>
        HttpResponse.json({ redirect_to: 'https://claude.ai/cb?error=access_denied' }),
      ),
    )
    const { user } = await renderApp(`/oauth/consent?request=${REQUEST}`)

    await user.click(await screen.findByRole('button', { name: 'Refuser' }))
    await waitFor(() => expect(leave).toHaveBeenCalledWith('https://claude.ai/cb?error=access_denied'))
  })

  it('explains an expired request or a refused app', async () => {
    server.use(...signedIn(), http.get(`${API}/oauth/requests/${REQUEST}`, () => problem(404, 'not_found')))
    await renderApp(`/oauth/consent?request=${REQUEST}`)
    expect(await screen.findByText(/Cette demande a expiré ou a déjà été traitée/)).toBeTruthy()
  })

  it('never shows the reason of an unknown error code verbatim', async () => {
    server.use(...signedIn())
    await renderApp('/oauth/consent?error=<script>')
    expect(await screen.findByText('Demande d’autorisation refusée.')).toBeTruthy()
  })

  it('asks to sign in first, then comes back to the request', async () => {
    const { router } = await renderApp(`/oauth/consent?request=${REQUEST}`)
    await waitFor(() => expect(router.currentRoute.value.path).toBe('/login'))
    expect(router.currentRoute.value.query.redirect).toBe(`/oauth/consent?request=${REQUEST}`)
  })
})

describe('connected apps', () => {
  it('lists the apps of the account and revokes one', async () => {
    let revoked = false
    server.use(
      ...signedIn(),
      http.get(`${API}/oauth/grants`, () =>
        HttpResponse.json(
          revoked
            ? []
            : [
                {
                  id: GRANT,
                  client_name: 'Claude',
                  client_id: 'https://claude.ai/oauth/mcp-oauth-client-metadata',
                  registration: 'metadata',
                  scopes: ['servers.read'],
                  created_at: '2026-10-27T10:00:00Z',
                  last_used_at: null,
                },
              ],
        ),
      ),
      http.delete(`${API}/oauth/grants/${GRANT}`, () => {
        revoked = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp('/account/applications')

    const card = (await screen.findByText('https://claude.ai/oauth/mcp-oauth-client-metadata')).closest('[data-slot="card"]') as HTMLElement
    expect(within(card).getByText('jamais utilisée')).toBeTruthy()
    await user.click(within(card).getByRole('button', { name: 'Révoquer' }))

    expect(await screen.findByText('Accès de Claude révoqué.')).toBeTruthy()
    expect(await screen.findByText('Aucune application connectée.')).toBeTruthy()
  })
})
