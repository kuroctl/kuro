// Infrastructure (ADR-0021) : vue d'ensemble (carte, indicateurs, derniers serveurs), carte du
// monde, régions, emplacement et enrôlement d'un nœud. MapLibre est simulé (`tests/support/maplibre.ts`).
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { Map } from '../support/maplibre'
import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, NODE_ID, serverDto } from '../support/servers'

const REGION_ID = '01a17b00-0000-7000-8000-000000000001'
const OTHER_NODE = '01a17a00-0000-7000-8000-000000000002'
const GiB = 1024 ** 3

const region: Schemas['RegionDto'] = {
  id: REGION_ID,
  slug: 'eu-west',
  name: 'Europe Ouest',
  latitude: 48.85,
  longitude: 2.35,
  nodes: { total: 1, online: 1, offline: 0, pending: 0, revoked: 0 },
  created_at: '2026-10-01T00:00:00Z',
}

const placed: Schemas['NodeDto'] = {
  ...node,
  region: { id: REGION_ID, slug: 'eu-west', name: 'Europe Ouest' },
  location: 'Roubaix',
  map_position: { latitude: 50.69, longitude: 3.17 },
  host: { daemon_version: '0.1.0', memory_bytes: 16 * GiB, cpu_cores: 8 },
}
const pending: Schemas['NodeDto'] = { id: OTHER_NODE, name: 'dev-2', status: 'pending', created_at: '2026-10-02T00:00:00Z', trials: false }

function setup(nodes = [placed, pending], regions = [region]) {
  server.use(
    ...signedIn(),
    // Avant `catalog()`, qui répond aussi à `/nodes` : le premier handler gagne.
    http.get(`${API}/nodes`, () => HttpResponse.json(nodes)),
    ...catalog(),
    http.get(`${API}/regions`, () => HttpResponse.json(regions)),
    http.get(`${API}/servers`, () =>
      HttpResponse.json([
        serverDto({ name: 'Ancien', created_at: '2026-10-01T00:00:00Z', state: 'running', memory_mb: 4096, disk_mb: 10240, disk_used_bytes: 5 * GiB }),
        serverDto({ id: '01a17a00-0000-7000-8000-0000000000bb', name: 'Récent', created_at: '2026-10-20T00:00:00Z', state: 'failed', memory_mb: 4096 }),
      ]),
    ),
  )
}

describe('infrastructure overview', () => {
  it('opens on the overview with useful metrics and the latest servers first', async () => {
    setup()
    await renderApp('/servers')

    const servers = await screen.findByText('Serveurs', { selector: '[data-metric="servers"] *' })
    const card = servers.closest<HTMLElement>('[data-metric]')!
    expect(within(card).getByText('2')).toBeTruthy()
    expect(within(card).getByText('1 en ligne')).toBeTruthy()
    const attention = document.querySelector<HTMLElement>('[data-metric="attention"]')!
    expect(within(attention).getByText('1')).toBeTruthy()
    const nodes = document.querySelector<HTMLElement>('[data-metric="nodes"]')!
    expect(await within(nodes).findByText('1 / 2')).toBeTruthy()
    expect(within(nodes).getByText('1 en attente d’enrôlement')).toBeTruthy()
    const memory = document.querySelector<HTMLElement>('[data-metric="memory"]')!
    expect(await within(memory).findByText(/sur 16/)).toBeTruthy()

    const latest = screen.getByText('Derniers serveurs créés').closest<HTMLElement>('[data-slot="card"]')!
    const names = within(latest)
      .getAllByRole('link')
      .map((link) => link.textContent ?? '')
      .filter((text) => /Ancien|Récent/.test(text))
    expect(names[0]).toContain('Récent')
    expect(screen.getByRole('button', { name: /Nouveau serveur/ })).toBeTruthy()
  })

  it('places the nodes on the map and points out those without a position', async () => {
    setup()
    await renderApp('/servers')

    const marker = await screen.findByLabelText('Dev 1')
    expect(marker.closest<HTMLElement>('[data-position]')?.dataset.position).toBe('50.69,3.17')
    expect(screen.getByText('1 nœud sans position : donnez-lui une région ou des coordonnées.')).toBeTruthy()
  })

  it('moves the servers list under its own entry', async () => {
    setup()
    await renderApp('/servers/list')

    expect(await screen.findByText('Ancien')).toBeTruthy()
    const nav = screen.getAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))!
    expect(within(nav).getByRole('link', { name: 'Serveurs' }).getAttribute('data-active')).toBe('true')
  })
})

describe('world map', () => {
  it('counts the nodes by status and filters by region', async () => {
    setup()
    const { user } = await renderApp('/servers/map')

    const legend = await screen.findByRole('list', { name: 'Légende' })
    await waitFor(() => expect(within(legend).getByText('En ligne').parentElement?.textContent).toContain('1'))
    expect(within(legend).getByText('En attente').parentElement?.textContent).toContain('1')

    await user.click(screen.getByRole('combobox', { name: 'Région' }))
    await user.click(await screen.findByRole('option', { name: 'Europe Ouest' }))
    await waitFor(() => expect(within(legend).getByText('En attente').parentElement?.textContent).toContain('0'))
  })
})

/** Première cellule des lignes de données, dans l'ordre affiché. */
function firstCells() {
  return screen
    .getAllByRole('row')
    .slice(1)
    .map((row) => within(row).getAllByRole('cell')[0]?.textContent?.trim())
}

/** Bouton d'un filtre à facettes (l'en-tête de colonne porte le même nom). */
function facet(name: string) {
  return screen.getAllByRole('button', { name }).find((button) => !button.closest('th'))!
}

describe('nodes list', () => {
  const OFFLINE_NODE = '01a17a00-0000-7000-8000-000000000003'
  const offline: Schemas['NodeDto'] = {
    id: OFFLINE_NODE,
    name: 'bravo',
    status: 'offline',
    region: { id: REGION_ID, slug: 'eu-west', name: 'Europe Ouest' },
    created_at: '2026-10-03T00:00:00Z',
    trials: false,
  }

  it('lists the nodes and opens a node on click', async () => {
    setup([placed, pending, offline])
    server.use(http.get(`${API}/nodes/${NODE_ID}`, () => HttpResponse.json(placed)))
    const { user, router } = await renderApp('/servers/nodes')

    await screen.findByText('dev-2')
    expect(firstCells()).toEqual(['bravo', 'Dev 1dev-1', 'dev-2'])
    await user.click(screen.getByText('Dev 1'))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/servers/nodes/${NODE_ID}`))
  })

  it('searches by name, keeping the search in the URL', async () => {
    setup([placed, pending, offline])
    const { user, router } = await renderApp('/servers/nodes')

    await user.type(await screen.findByRole('searchbox', { name: 'Rechercher un nœud…' }), 'dev-1')
    await waitFor(() => expect(firstCells()).toEqual(['Dev 1dev-1']))
    expect(router.currentRoute.value.query.q).toBe('dev-1')
    expect(screen.getByText('1 résultat')).toBeTruthy()
  })

  it('filters by status and region, counting with the other filters applied', async () => {
    setup([placed, pending, offline])
    const { user, router } = await renderApp('/servers/nodes')

    await screen.findByText('dev-2')
    await user.click(facet('Région'))
    let popover = await screen.findByRole('dialog')
    expect(within(popover).getByText('Europe Ouest').parentElement?.textContent).toContain('2')
    expect(within(popover).getByText('Sans région').parentElement?.textContent).toContain('1')
    await user.click(within(popover).getByText('Europe Ouest'))
    await waitFor(() => expect(firstCells()).toEqual(['bravo', 'Dev 1dev-1']))
    expect(router.currentRoute.value.query.region).toBe('eu-west')
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())

    await user.click(facet('État'))
    popover = await screen.findByRole('dialog')
    // Compté dans la région choisie : le nœud en attente, sans région, n'y figure pas.
    expect(within(popover).getByText('En attente').parentElement?.textContent).toContain('0')
    expect(within(popover).getByText('En ligne').parentElement?.textContent).toContain('1')
    await user.click(within(popover).getByText('Hors ligne'))
    await waitFor(() => expect(firstCells()).toEqual(['bravo']))
    expect(router.currentRoute.value.query.status).toBe('offline')
  })

  it('sorts by a column, in both directions', async () => {
    setup([placed, pending, offline])
    const { user, router } = await renderApp('/servers/nodes')

    await screen.findByText('dev-2')
    const status = screen.getAllByRole('columnheader').find((header) => header.textContent?.includes('État'))!
    await user.click(within(status).getByRole('button'))
    await waitFor(() => expect(firstCells()).toEqual(['bravo', 'Dev 1dev-1', 'dev-2']))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'status', order: 'asc' })

    const name = screen.getAllByRole('columnheader').find((header) => header.textContent?.includes('Nom'))!
    await user.click(within(name).getByRole('button'))
    await user.click(within(name).getByRole('button'))
    await waitFor(() => expect(firstCells()).toEqual(['dev-2', 'Dev 1dev-1', 'bravo']))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'name', order: 'desc' })
  })
})

describe('regions', () => {
  const other: Schemas['RegionDto'] = {
    ...region,
    id: '01a17b00-0000-7000-8000-000000000002',
    slug: 'us-east',
    name: 'Amérique Est',
    nodes: { total: 3, online: 2, offline: 1, pending: 0, revoked: 0 },
  }

  it('searches the regions by name or identifier', async () => {
    setup(undefined, [region, other])
    const { user } = await renderApp('/servers/regions')

    await screen.findByText('Europe Ouest')
    expect(firstCells()).toEqual(['Amérique Est', 'Europe Ouest'])
    await user.type(screen.getByRole('searchbox', { name: 'Rechercher une région…' }), 'eu-')
    await waitFor(() => expect(firstCells()).toEqual(['Europe Ouest']))
  })

  it('sorts the regions by node count', async () => {
    setup(undefined, [region, other])
    const { user, router } = await renderApp('/servers/regions')

    await screen.findByText('Europe Ouest')
    const nodes = screen.getAllByRole('columnheader').find((header) => header.textContent?.includes('Nœuds'))!
    await user.click(within(nodes).getByRole('button'))
    await user.click(within(nodes).getByRole('button'))
    await waitFor(() => expect(firstCells()).toEqual(['Amérique Est', 'Europe Ouest']))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'nodes', order: 'desc' })
    await user.click(within(nodes).getByRole('button'))
    await waitFor(() => expect(firstCells()).toEqual(['Europe Ouest', 'Amérique Est']))
  })

  it('lists the regions with their nodes', async () => {
    setup()
    await renderApp('/servers/regions')

    expect(await screen.findByText('Europe Ouest')).toBeTruthy()
    expect(screen.getByText('eu-west')).toBeTruthy()
    expect(screen.getByText('1 en ligne sur 1')).toBeTruthy()
  })

  it('creates a region centred where the map is clicked', async () => {
    setup()
    let body: unknown
    server.use(
      http.post(`${API}/regions`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(region, { status: 201 })
      }),
    )
    const { user } = await renderApp('/servers/regions')

    await user.click(await screen.findByRole('button', { name: 'Nouvelle région' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Amérique Est')
    await user.type(within(dialog).getByLabelText('Identifiant'), 'us-east')
    Map.last!.click(40.712345678, -74.006)
    await user.click(within(dialog).getByRole('button', { name: 'Créer la région' }))

    await waitFor(() => expect(body).toEqual({ name: 'Amérique Est', slug: 'us-east', latitude: 40.71235, longitude: -74.006 }))
  })

  it('explains that a region still holding nodes cannot be deleted', async () => {
    setup()
    server.use(http.delete(`${API}/regions/${REGION_ID}`, () => problem(409, 'region_not_empty')))
    const { user } = await renderApp('/servers/regions')

    await user.click(await screen.findByRole('button', { name: 'Actions sur Europe Ouest' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Supprimer' }))
    await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Supprimer' }))

    expect(await screen.findByText('La région contient encore des nœuds.')).toBeTruthy()
  })

  it('offers no change without the regions permission', async () => {
    setup()
    server.use(http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['nodes.read'] })))
    await renderApp('/servers/regions')

    expect(await screen.findByText('Europe Ouest')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Nouvelle région' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Actions sur Europe Ouest' })).toBeNull()
  })
})

describe('node location', () => {
  it('sends only the changed location fields', async () => {
    setup()
    server.use(http.get(`${API}/nodes/${NODE_ID}`, () => HttpResponse.json(placed)))
    let body: unknown
    server.use(
      http.patch(`${API}/nodes/${NODE_ID}`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...placed, location: 'Gravelines', coordinates: { latitude: 51, longitude: 2.1 } })
      }),
    )
    const { user } = await renderApp(`/servers/nodes/${NODE_ID}`)

    const place = await screen.findByLabelText('Lieu')
    await user.clear(place)
    await user.type(place, 'Gravelines')
    Map.last!.click(51, 2.1)
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ location: 'Gravelines', coordinates: { latitude: 51, longitude: 2.1 } }))
  })
})

describe('node enrollment', () => {
  const ticket = (overrides: Partial<Schemas['NodeDto']> = {}): Schemas['NodeTicketDto'] => ({
    node: { ...pending, ...overrides },
    enrollment: { token: 'kuro_enr_secret', expires_in: 900, ca_fingerprint: 'ab12cd34', daemon_port: 8443 },
  })

  it('adds a node and shows its token apart from the install command', async () => {
    setup()
    let body: unknown
    server.use(
      http.post(`${API}/nodes`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(ticket(), { status: 201 })
      }),
      http.get(`${API}/nodes/${OTHER_NODE}`, () => HttpResponse.json(pending)),
    )
    const { user, router } = await renderApp('/servers/nodes')

    await user.click(await screen.findByRole('button', { name: 'Ajouter un nœud' }))
    await user.type(screen.getByLabelText('Nom technique'), 'dev-2')
    await user.click(screen.getByRole('button', { name: 'Créer le nœud' }))

    expect(await screen.findByTestId('enroll-token')).toHaveProperty('textContent', 'kuro_enr_secret')
    expect(body).toEqual({ name: 'dev-2', display_name: null })
    expect(screen.getByText(/valable 15 min/)).toBeTruthy()
    const command = screen.getByTestId('enroll-command').textContent ?? ''
    expect(command).toContain('install.sh --api https://localhost:8443 --ca-fingerprint sha256:ab12cd34')
    expect(command).toContain('read -rs KURO_ENROLL_TOKEN')
    expect(command).not.toContain('kuro_enr_secret')

    await user.click(screen.getByRole('button', { name: 'Voir le nœud' }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/servers/nodes/${OTHER_NODE}`))
  })

  it('shows a refused name under its field', async () => {
    setup()
    server.use(http.post(`${API}/nodes`, () => problem(422, 'invalid', { fields: [{ field: 'name', code: 'invalid_node_name', params: {} }] })))
    const { user } = await renderApp('/servers/nodes')

    await user.click(await screen.findByRole('button', { name: 'Ajouter un nœud' }))
    await user.type(screen.getByLabelText('Nom technique'), 'Dev 2')
    await user.click(screen.getByRole('button', { name: 'Créer le nœud' }))

    expect(await screen.findByText('Nom de nœud invalide (minuscules, chiffres et tirets).')).toBeTruthy()
    expect(screen.queryByTestId('enroll-token')).toBeNull()
  })

  it('issues a new token for a pending node', async () => {
    setup()
    let calls = 0
    server.use(
      http.get(`${API}/nodes/${OTHER_NODE}`, () => HttpResponse.json(pending)),
      http.post(`${API}/nodes/${OTHER_NODE}/enrollment-token`, () => {
        calls += 1
        return HttpResponse.json(ticket())
      }),
    )
    const { user } = await renderApp(`/servers/nodes/${OTHER_NODE}`)

    await user.click(await screen.findByRole('button', { name: 'Émettre un jeton' }))

    expect(await screen.findByTestId('enroll-token')).toHaveProperty('textContent', 'kuro_enr_secret')
    expect(calls).toBe(1)
    expect(screen.getByRole('button', { name: 'Émettre un autre jeton' })).toBeTruthy()
  })

  it('offers no enrollment for an online node', async () => {
    setup()
    server.use(http.get(`${API}/nodes/${NODE_ID}`, () => HttpResponse.json(placed)))
    await renderApp(`/servers/nodes/${NODE_ID}`)
    await screen.findByText('Emplacement')
    expect(screen.queryByText('Enrôlement')).toBeNull()
  })

  it('hides adding and enrolling without the nodes permissions', async () => {
    setup()
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['nodes.read'] })),
      http.get(`${API}/nodes/${OTHER_NODE}`, () => HttpResponse.json(pending)),
    )
    const { router } = await renderApp('/servers/nodes')
    await screen.findByText('dev-2')
    expect(screen.queryByRole('button', { name: 'Ajouter un nœud' })).toBeNull()

    await router.push(`/servers/nodes/${OTHER_NODE}`)
    await screen.findByText('Emplacement')
    expect(screen.queryByText('Enrôlement')).toBeNull()
  })
})
