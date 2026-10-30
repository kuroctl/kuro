// Liste des serveurs en data table (ADR-0025) : critères dans l'URL et envoyés à l'API, filtre
// rapide par jeu, sélection étendue à tous les résultats, action groupée et son résultat.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, serverDto } from '../support/servers'

const ids = ['01a17a00-0000-7000-8000-000000000100', '01a17a00-0000-7000-8000-000000000101', '01a17a00-0000-7000-8000-000000000102'] as const

function page(params: URLSearchParams): Schemas['ServerPageDto'] {
  const items = [serverDto({ id: ids[0], name: 'Alpha' }), serverDto({ id: ids[1], name: 'Bravo' })]
  return {
    items,
    total: 3,
    page: Number(params.get('page') ?? 1),
    per_page: 2,
    facets: {
      prefab: [
        { value: 'minecraft-java', label: '1.0.0', count: 2 },
        { value: 'terraria', label: '1.0.0', count: 1 },
      ],
      node: [],
      state: [{ value: 'offline', label: 'offline', count: 3 }],
      upgrade: 1,
      suspended: 0,
    },
    upgrades: { [ids[0]]: '1.1.0' },
    ids: params.get('all_ids') === 'true' ? [...ids] : null,
  }
}

function setup() {
  const requests: URLSearchParams[] = []
  // Avant `signedIn()`, qui simule une recherche générique : le premier gestionnaire l'emporte.
  server.use(
    http.get(`${API}/servers/search`, ({ request }) => {
      const params = new URL(request.url).searchParams
      requests.push(params)
      return HttpResponse.json(page(params))
    }),
    ...signedIn(),
    ...catalog(),
  )
  return requests
}

describe('server list', () => {
  it('sends the criteria to the API and keeps them in the URL', async () => {
    const requests = setup()
    const { user, router } = await renderApp('/servers/list')

    expect(await screen.findByText('Alpha')).toBeTruthy()
    // Mise à jour disponible signalée sur la ligne.
    expect(screen.getByTitle('Version 1.1.0 disponible')).toBeTruthy()

    await user.click(screen.getByRole('button', { name: /terraria/ }))
    await waitFor(() => expect(router.currentRoute.value.query.prefab).toBe('terraria'))
    await waitFor(() => expect(requests.at(-1)?.get('prefab')).toBe('terraria'))

    await user.click(screen.getByRole('button', { name: /^Nom/ }))
    await waitFor(() => expect(requests.at(-1)?.get('order')).toBe('desc'))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'name', order: 'desc', prefab: 'terraria' })
  })

  it('selects every result across pages, then runs a bulk action on them', async () => {
    setup()
    let body: unknown
    server.use(
      http.post(`${API}/servers/bulk`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json([
          { server_id: ids[0], ok: true, error: null },
          { server_id: ids[1], ok: true, error: null },
          { server_id: ids[2], ok: false, error: 'server_suspended' },
        ])
      }),
    )
    const { user } = await renderApp('/servers/list')

    await user.click(await screen.findByRole('checkbox', { name: 'Sélectionner la page' }))
    const bar = screen.getByRole('region', { name: 'Sélection' })
    expect(within(bar).getByText('2 sélectionnés')).toBeTruthy()
    await user.click(within(bar).getByRole('button', { name: 'Sélectionner les 3 résultats' }))
    expect(within(bar).getByText('Les 3 résultats sont sélectionnés')).toBeTruthy()

    await user.click(within(bar).getByRole('button', { name: /Actions/ }))
    await user.click(await screen.findByRole('menuitem', { name: 'Démarrer' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('Démarrer 3 serveurs.')).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Appliquer à 3 serveurs' }))

    await waitFor(() => expect(body).toEqual({ action: 'start', servers: ids, backup: false }))
    expect(await within(dialog).findByText('2 réussi(s), 1 refusé(s) :')).toBeTruthy()
    expect(within(dialog).getByText('Ce serveur est suspendu.')).toBeTruthy()
  })
})
