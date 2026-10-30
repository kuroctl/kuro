// DNS des serveurs et des nœuds (ADR-0026) : configuration de la zone, enregistrements, adresse
// publique d'un nœud, nom d'un serveur et adresse affichée.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, NODE_ID, SERVER_ID, serverDto } from '../support/servers'

type DnsSettings = Schemas['DnsSettingsDto']

const zone = { base_domain: 'play.example.com', nodes_label: 'nodes' }

const settings: DnsSettings = {
  configured: true,
  provider: 'cloudflare',
  base_domain: 'play.example.com',
  nodes_label: 'nodes',
  ttl: 300,
  enabled: true,
  has_token: true,
  records: [
    {
      id: '01a17a00-0000-7000-8000-00000000d001',
      owner_kind: 'node',
      owner_id: NODE_ID,
      kind: 'A',
      name: 'dev-1.nodes.play.example.com',
      content: '203.0.113.10',
      published: true,
      synced_at: '2026-10-28T10:00:00Z',
      error: null,
    },
    {
      id: '01a17a00-0000-7000-8000-00000000d002',
      owner_kind: 'server',
      owner_id: SERVER_ID,
      kind: 'CNAME',
      name: 'survie.play.example.com',
      content: 'dev-1.nodes.play.example.com',
      published: false,
      synced_at: null,
      error: 'record quota exceeded',
    },
  ],
}

/** Page des paramètres DNS ; le propriétaire des enregistrements vient des listes. */
function settingsPage(value: DnsSettings = settings) {
  server.use(
    ...signedIn(),
    http.get(`${API}/settings/dns`, () => HttpResponse.json(value)),
    http.get(`${API}/nodes`, () => HttpResponse.json([node])),
    http.get(`${API}/servers`, () => HttpResponse.json([serverDto()])),
  )
}

describe('dns settings', () => {
  it('sends the token only when typed', async () => {
    const bodies: unknown[] = []
    settingsPage()
    server.use(
      http.put(`${API}/settings/dns`, async ({ request }) => {
        bodies.push(await request.json())
        return HttpResponse.json(settings)
      }),
    )
    const { user } = await renderApp('/settings/dns')

    const token = await screen.findByLabelText('Jeton d’API')
    expect(token.getAttribute('type')).toBe('password')
    expect(token.getAttribute('placeholder')).toBe('enregistré')
    const domain = screen.getByLabelText('Domaine des serveurs')
    await user.clear(domain)
    await user.type(domain, 'mc.example.com')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(bodies).toHaveLength(1))
    expect(bodies[0]).toEqual({ provider: 'cloudflare', base_domain: 'mc.example.com', nodes_label: 'nodes', ttl: 300, enabled: true })
    expect(await screen.findByText('Configuration DNS enregistrée.')).toBeTruthy()

    await user.type(screen.getByLabelText('Jeton d’API'), 'cf-token')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(bodies).toHaveLength(2))
    expect(bodies[1]).toMatchObject({ token: 'cf-token' })
  })

  it('shows the refusal of the provider on the page', async () => {
    settingsPage()
    server.use(http.put(`${API}/settings/dns`, () => problem(422, 'dns_provider_unauthorized')))
    const { user } = await renderApp('/settings/dns')

    await user.type(await screen.findByLabelText('Jeton d’API'), 'wrong-token')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    const alert = await screen.findByRole('alert')
    expect(alert.textContent).toContain('Le fournisseur DNS a refusé le jeton')
  })

  it('shows a field refusal under the domain', async () => {
    settingsPage()
    server.use(
      http.put(`${API}/settings/dns`, () =>
        problem(422, 'validation_failed', { fields: [{ field: 'base_domain', code: 'invalid_domain' }] }),
      ),
    )
    const { user } = await renderApp('/settings/dns')

    const domain = await screen.findByLabelText('Domaine des serveurs')
    await user.clear(domain)
    await user.type(domain, 'not a domain')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    expect(await screen.findByText('Nom de domaine invalide (ex. play.example.com).')).toBeTruthy()
  })

  it('lists the records with their status and the error of a failed one', async () => {
    settingsPage()
    const { user } = await renderApp('/settings/dns')

    const failed = (await screen.findByText('survie.play.example.com')).closest('tr')!
    expect(within(failed).getByText('Erreur')).toBeTruthy()
    expect(within(failed).getByText('record quota exceeded')).toBeTruthy()
    expect(await within(failed).findByText('Survie')).toBeTruthy()
    const published = screen.getByText('203.0.113.10').closest('tr')!
    expect(within(published).getByText('Publié')).toBeTruthy()
    expect(await within(published).findByText('Dev 1')).toBeTruthy()

    // Facette « Statut » : seulement les enregistrements en erreur.
    await user.click(screen.getByRole('button', { name: /Statut/ }))
    await user.click(await screen.findByRole('checkbox', { name: /Erreur/ }))
    await waitFor(() => expect(screen.queryByText('203.0.113.10')).toBeNull())
    expect(screen.getByText('survie.play.example.com')).toBeTruthy()
  })

  it('removes the zone after a confirmation', async () => {
    settingsPage()
    let removed = false
    server.use(
      http.delete(`${API}/settings/dns`, () => {
        removed = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp('/settings/dns')

    await user.click(await screen.findByRole('button', { name: 'Supprimer la zone' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog.textContent).toContain('Kuro supprime chez le fournisseur tous les enregistrements qu’il a créés')
    await user.click(within(dialog).getByRole('button', { name: 'Supprimer la zone' }))

    await waitFor(() => expect(removed).toBe(true))
  })

  it('is listed in the settings menu only with the permission', async () => {
    server.use(...signedIn(), http.get(`${API}/settings`, () => problem(403, 'forbidden')))
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['system.settings.read', 'audit.read'] }),
      ),
    )
    await renderApp('/settings/audit')

    expect(await screen.findByRole('link', { name: /Journal d’audit/ })).toBeTruthy()
    expect(screen.queryByRole('link', { name: /DNS/ })).toBeNull()
  })
})

describe('node public address', () => {
  it('saves the address and shows the node name', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/nodes/${NODE_ID}`, () => HttpResponse.json({ ...node, public_ipv4: '203.0.113.10' })),
      http.get(`${API}/nodes/${NODE_ID}/allocations`, () => HttpResponse.json([])),
      http.get(`${API}/regions`, () => HttpResponse.json([])),
      http.put(`${API}/nodes/${NODE_ID}/address`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 204 })
      }),
    )
    server.use(http.get(`${API}/dns`, () => HttpResponse.json(zone)))
    const { user } = await renderApp(`/servers/nodes/${NODE_ID}`)

    expect(await screen.findByText('dev-1.nodes.play.example.com')).toBeTruthy()
    const ipv4 = screen.getByLabelText('IPv4')
    expect((ipv4 as HTMLInputElement).value).toBe('203.0.113.10')
    await user.clear(ipv4)
    await user.type(ipv4, '198.51.100.7')
    await user.type(screen.getByLabelText('IPv6 (facultative)'), '2001:db8::7')
    await user.click(screen.getByRole('button', { name: 'Enregistrer l’adresse' }))

    await waitFor(() => expect(body).toEqual({ ipv4: '198.51.100.7', ipv6: '2001:db8::7' }))
    expect(await screen.findByText('Adresse publique enregistrée.')).toBeTruthy()
  })
})

describe('server dns name', () => {
  /** Vue d'ensemble sans la console (pas de WebSocket) mais avec le droit de modifier. */
  function overview(label: string | null = 'survie') {
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([serverDto({ dns_label: label })])),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ dns_label: label }))),
    )
    server.use(
      http.get(`${API}/dns`, () => HttpResponse.json(zone)),
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read', 'servers.update'] }),
      ),
    )
  }

  it('shows the address by name and edits the label, with the refusal under the field', async () => {
    overview()
    const bodies: unknown[] = []
    server.use(
      http.put(`${API}/servers/${SERVER_ID}/dns`, async ({ request }) => {
        const body = (await request.json()) as { label: string | null }
        bodies.push(body)
        if (body.label === 'pris') return problem(422, 'validation_failed', { fields: [{ field: 'label', code: 'dns_label_taken' }] })
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByText('survie.play.example.com:25565')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Copier survie.play.example.com:25565' })).toBeTruthy()

    await user.click(screen.getByRole('button', { name: 'Renommer' }))
    const field = await screen.findByLabelText('Nom')
    expect((field as HTMLInputElement).value).toBe('survie')
    await user.clear(field)
    await user.type(field, 'pris')
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Enregistrer' }))
    expect(await screen.findByText('Ce nom est déjà utilisé par un autre serveur.')).toBeTruthy()

    await user.clear(field)
    await user.type(field, 'creatif')
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Enregistrer' }))
    await waitFor(() => expect(bodies.at(-1)).toEqual({ label: 'creatif' }))
    expect(await screen.findByText('Nom DNS enregistré.')).toBeTruthy()
  })

  it('keeps the IP address without a name, and shows the name in the list', async () => {
    overview(null)
    await renderApp(`/servers/${SERVER_ID}`)
    expect(await screen.findByText('0.0.0.0:25565')).toBeTruthy()
    expect(await screen.findByRole('button', { name: 'Nommer' })).toBeTruthy()
  })

  it('shows the hostname in the servers list', async () => {
    overview()
    await renderApp('/servers/list')
    expect(await screen.findByText('survie.play.example.com:25565')).toBeTruthy()
  })
})
