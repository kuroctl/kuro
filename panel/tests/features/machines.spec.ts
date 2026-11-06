// Nœuds à la demande (ADR-0065) : page des machines (aucun fournisseur → Réglages d'abord,
// location, retour), Réglages › Machines (jeton, régions), Réglages › Tâches (simulation,
// lancement, ADR-0062), création d'un serveur sur une nouvelle machine, couleur sur la carte.
import { screen, waitFor } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, NODE_ID, SERVER_ID, serverDto } from '../support/servers'

const REGION_ID = '01a17a00-0000-7000-8000-00000000e001'
const MACHINE_ID = '01a17a00-0000-7000-8000-00000000e002'

const settings: Schemas['MachinesSettings'] = {
  api_url: 'https://panel.example.com:8443',
  port_range: '25565-25600',
  providers: [{ name: 'hetzner', configured: true, updated_at: '2026-11-06T10:00:00Z' }],
  regions: [{ region_id: REGION_ID, slug: 'eu-west', name: 'Europe Ouest', provider: 'hetzner', location: 'fsn1' }],
}

const offers: Schemas['Offer'][] = [
  { name: 'cx23', vcpus: 2, memory_mb: 4096, disk_gb: 40, hourly_cost: 600, monthly_cost: 379 },
  { name: 'cpx31', vcpus: 4, memory_mb: 8192, disk_gb: 160, hourly_cost: 2000, monthly_cost: 1249 },
]

const machine: Schemas['MachineView'] = {
  id: MACHINE_ID,
  provider: 'hetzner',
  reference: '42',
  offer: 'cx23',
  location: 'fsn1',
  region_id: REGION_ID,
  region_name: 'Europe Ouest',
  hourly_cost: 600,
  monthly_cost: 379,
  node_id: NODE_ID,
  node_name: 'm-0123456789ab',
  node_status: 'online',
  ipv4: '203.0.113.9',
  has_snapshot: false,
  claim: null,
  state: 'ready',
  server_id: null,
  flag: null,
  created_at: '2026-11-06T10:00:00Z',
}

const sweep: Schemas['TaskStatus'] = {
  name: 'machines.sweep',
  period_secs: 60,
  dry_run: true,
  running: false,
  finished_at: '2026-11-06T10:05:00Z',
  started_at: '2026-11-06T10:04:59Z',
  outcome: 'ok',
  report: { dry_run: true, orphans: 2 },
  last_dry_run: true,
  duration_ms: 120,
}

function page(value: Partial<Schemas['MachinesPage']> = {}) {
  return http.get(`${API}/machines`, () => HttpResponse.json({ configured: true, machines: [machine], sweep, ...value }))
}

describe('machines page', () => {
  it('sends to the settings first when no provider is configured', async () => {
    server.use(page({ configured: false, machines: [] }), ...signedIn())
    const { router, user } = await renderApp('/servers/machines')

    expect(await screen.findByText('Aucun fournisseur configuré')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Louer une machine' })).toBeNull()
    await user.click(screen.getByRole('link', { name: 'Configurer les fournisseurs' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/settings/machines'))
  })

  it('lists machines with their state and the last sweep, and rents one', async () => {
    let body: unknown
    server.use(
      page(),
      http.get(`${API}/settings/machines`, () => HttpResponse.json(settings)),
      http.get(`${API}/machines/offers`, () => HttpResponse.json(offers)),
      http.post(`${API}/machines`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...machine, state: 'creating' }, { status: 201 })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/servers/machines')

    expect(await screen.findByText('m-0123456789ab')).toBeTruthy()
    expect(screen.getByText('En marche')).toBeTruthy()
    expect(screen.getByText('Hetzner Cloud · fsn1 · 203.0.113.9')).toBeTruthy()
    expect(screen.getByText('Orphelines')).toBeTruthy()

    await user.click(screen.getByRole('button', { name: 'Louer une machine' }))
    await user.click(await screen.findByRole('combobox', { name: 'Offre' }))
    await user.click(await screen.findByRole('option', { name: /cpx31 · 4 vCPU · 8 Go · 160 Go/ }))
    await user.click(screen.getByRole('button', { name: 'Louer' }))

    await waitFor(() => expect(body).toEqual({ region_id: REGION_ID, offer: 'cpx31' }))
  })

  it('releases a machine after naming it', async () => {
    let released = false
    server.use(
      page(),
      http.delete(`${API}/machines/${MACHINE_ID}`, () => {
        released = true
        return new HttpResponse(null, { status: 204 })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/servers/machines')

    await user.click(await screen.findByRole('button', { name: 'Rendre' }))
    expect(await screen.findByText('Rendre la machine m-0123456789ab ?')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Rendre la machine' }))

    await waitFor(() => expect(released).toBe(true))
  })
})

describe('machines settings', () => {
  it('saves a provider token without ever showing it', async () => {
    let token: unknown
    server.use(
      http.get(`${API}/settings/machines`, () => HttpResponse.json(settings)),
      http.get(`${API}/settings/machines/providers/hetzner/locations`, () => HttpResponse.json([])),
      http.put(`${API}/settings/machines/providers/hetzner`, async ({ request }) => {
        token = await request.json()
        return HttpResponse.json({ settings, created_regions: ['Helsinki (FI)', 'Ashburn, VA (US)'] })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/settings/machines')

    expect(await screen.findByRole('tab', { name: 'Hetzner Cloud' })).toBeTruthy()
    const input = screen.getByLabelText('Jeton d’API')
    expect(input.getAttribute('placeholder')).toBe('enregistré')
    await user.type(input, 'hz-secret')
    await user.click(screen.getByRole('button', { name: 'Enregistrer le jeton' }))

    await waitFor(() => expect(token).toEqual({ token: 'hz-secret' }))
    expect(await screen.findByText(/Régions créées, désactivées : Helsinki \(FI\), Ashburn, VA \(US\)/)).toBeTruthy()
  })

  it('saves the regions with their provider and location', async () => {
    let body: unknown
    server.use(
      http.get(`${API}/settings/machines`, () => HttpResponse.json(settings)),
      http.get(`${API}/settings/machines/providers/hetzner/locations`, () =>
        HttpResponse.json([
          { name: 'fsn1', city: 'Falkenstein', country: 'DE', latitude: 50.47, longitude: 12.37 },
          { name: 'nbg1', city: 'Nuremberg', country: 'DE', latitude: 49.45, longitude: 11.08 },
        ]),
      ),
      http.put(`${API}/settings/machines`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(settings)
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/settings/machines')

    const location = await screen.findByRole('combobox', { name: 'Emplacement de Europe Ouest' })
    await waitFor(() => expect(location.textContent).toContain('Falkenstein (DE) · fsn1'))
    await user.click(location)
    await user.click(await screen.findByRole('option', { name: 'Nuremberg (DE) · nbg1' }))
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({
        api_url: 'https://panel.example.com:8443',
        port_range: '25565-25600',
        regions: [{ region_id: REGION_ID, provider: 'hetzner', location: 'nbg1' }],
      }),
    )
  })
})

describe('location regions', () => {
  it('creates the regions of the provider locations without retyping the token', async () => {
    let called = false
    server.use(
      http.get(`${API}/settings/machines`, () => HttpResponse.json(settings)),
      http.get(`${API}/settings/machines/providers/hetzner/locations`, () => HttpResponse.json([])),
      http.post(`${API}/settings/machines/providers/hetzner/regions`, () => {
        called = true
        return HttpResponse.json({ settings, created_regions: ['Helsinki (FI)'] })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/settings/machines')

    await user.click(await screen.findByRole('button', { name: 'Créer les régions des emplacements' }))

    await waitFor(() => expect(called).toBe(true))
    expect(await screen.findByText(/Région créée, désactivée : Helsinki \(FI\)/)).toBeTruthy()
  })
})

describe('background tasks', () => {
  it('switches the simulation off and runs a task now', async () => {
    const calls: string[] = []
    server.use(
      http.get(`${API}/settings/tasks`, () => HttpResponse.json([sweep])),
      http.put(`${API}/settings/tasks/machines.sweep`, async ({ request }) => {
        calls.push(`dry_run=${((await request.json()) as { dry_run: boolean }).dry_run}`)
        return HttpResponse.json({ ...sweep, dry_run: false })
      }),
      http.post(`${API}/settings/tasks/machines.sweep/run`, () => {
        calls.push('run')
        return new HttpResponse(null, { status: 202 })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/settings/tasks')

    expect(await screen.findByText('Ménage des machines louées')).toBeTruthy()
    await user.click(screen.getByRole('switch', { name: 'Simulation' }))
    await waitFor(() => expect(calls).toContain('dry_run=false'))
    await user.click(screen.getByRole('button', { name: 'Lancer maintenant' }))

    await waitFor(() => expect(calls).toContain('run'))
  })
})

describe('create a server on a new machine', () => {
  it('rents a machine that receives the server once its node is online', async () => {
    let body: Record<string, unknown> | undefined
    server.use(
      http.get(`${API}/settings/machines`, () => HttpResponse.json(settings)),
      http.get(`${API}/machines/offers`, () => HttpResponse.json(offers)),
      http.post(`${API}/machines`, async ({ request }) => {
        body = (await request.json()) as Record<string, unknown>
        return HttpResponse.json({ ...machine, state: 'creating', server_id: SERVER_ID, server_state: 'waiting_node' }, { status: 201 })
      }),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'waiting_node' }))),
      page(),
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
    )
    const { router, user } = await renderApp('/servers/new?prefab=minecraft-java')

    await user.type(await screen.findByLabelText('Nom'), 'Survie')
    await user.click(screen.getByRole('combobox', { name: 'Nœud' }))
    await user.click(await screen.findByRole('option', { name: 'Nouvelle machine louée' }))
    await user.click(await screen.findByRole('combobox', { name: 'Offre' }))
    await user.click(await screen.findByRole('option', { name: /cx23/ }))
    expect(screen.queryByLabelText('CPU (cœurs)')).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    await user.click(await screen.findByRole('button', { name: 'Déployer' }))

    // Le serveur existe déjà : sa fiche le montre en attente de sa machine.
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/servers/${SERVER_ID}`))
    expect(await screen.findByText(/Le serveur attend sa machine/)).toBeTruthy()
    expect(body).toMatchObject({
      region_id: REGION_ID,
      offer: 'cx23',
      server: { name: 'Survie', prefab_id: 'minecraft-java' },
    })
    // La machine entière : ni nœud ni ressources choisis, et pas de champ pour les saisir.
    for (const field of ['node_id', 'memory_mb', 'cpu_percent', 'disk_mb']) expect(body?.server).not.toHaveProperty(field)
  })

  it('never offers a reserved node', async () => {
    server.use(
      http.get(`${API}/nodes`, () => HttpResponse.json([node, { ...node, id: MACHINE_ID, display_name: 'Réservé', reserved: true }])),
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
    )
    const { user } = await renderApp('/servers/new?prefab=minecraft-java')

    await user.click(await screen.findByRole('combobox', { name: 'Nœud' }))
    expect(await screen.findByRole('option', { name: 'Dev 1' })).toBeTruthy()
    expect(screen.queryByRole('option', { name: 'Réservé' })).toBeNull()
    expect(screen.queryByRole('option', { name: 'Nouvelle machine louée' })).toBeNull()
  })
})

describe('world map', () => {
  it('rings rented machines with their own color and counts them', async () => {
    server.use(
      http.get(`${API}/nodes`, () =>
        HttpResponse.json([{ ...node, machine_id: MACHINE_ID, map_position: { latitude: 50.1, longitude: 8.6 } }]),
      ),
      http.get(`${API}/regions`, () => HttpResponse.json([])),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
      ...signedIn(),
    )
    await renderApp('/servers/map')

    const legend = await screen.findByRole('list', { name: 'Légende' })
    await waitFor(() => expect(legend.querySelector('[data-status="machine"]')?.textContent).toMatch(/Machine louée\s*1/))
  })
})

describe('server waiting for its machine', () => {
  it('shows the machine of a waiting server and a translated failure', async () => {
    server.use(
      page({ machines: [{ ...machine, server_id: SERVER_ID, server_name: 'Survie', server_state: 'waiting_node' }] }),
      ...signedIn(),
    )
    await renderApp('/servers/machines')

    expect(await screen.findByRole('link', { name: 'Survie' })).toBeTruthy()
    expect(screen.getByText('En attente de la machine')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Mettre en sommeil' }).hasAttribute('disabled')).toBe(true)
  })

  it('explains why the server failed when its machine never came', async () => {
    server.use(
      http.get(`${API}/servers/${SERVER_ID}`, () =>
        HttpResponse.json(serverDto({ state: 'failed', state_reason: 'machine_not_ready_in_time' })),
      ),
      ...signedIn(),
      ...catalog(),
    )
    await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByText(/ne s’est jamais connectée \(45 min\)/)).toBeTruthy()
  })
})
