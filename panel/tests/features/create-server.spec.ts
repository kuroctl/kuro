// Création d'un serveur, première étape : catalogue des prefabs (recherche, catégories, tri,
// pagination) et arrivée directe sur un prefab par l'URL.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, prefabSummary } from '../support/servers'

function prefab(id: string, name: string, extra: Partial<Schemas['PrefabSummaryDto']> = {}): Schemas['PrefabSummaryDto'] {
  return { ...prefabSummary, id, names: { fr: name, en: name }, descriptions: { fr: `Serveur ${name}.` }, ...extra }
}

const wordpress = prefab('wordpress', 'WordPress', { category: 'web', tags: ['cms', 'php'] })
const valheim = prefab('valheim', 'Valheim', { tags: ['survival', 'steam'], servers: 5 })
const enshrouded = prefab('enshrouded', 'Énshrouded', { tags: ['survival', 'steam'] })

function setup(prefabs: Schemas['PrefabSummaryDto'][]) {
  server.use(
    ...signedIn(),
    http.get(`${API}/prefabs`, () => HttpResponse.json(prefabs)),
    ...catalog(),
    http.get(`${API}/servers`, () => HttpResponse.json([])),
  )
}

const shown = () => [...document.querySelectorAll('[data-prefab]')].map((item) => item.getAttribute('data-prefab'))

describe('create server: prefab catalog', () => {
  it('opens on its own page from the server list', async () => {
    setup([prefabSummary])
    const { router, user } = await renderApp('/servers/list')

    await user.click(await screen.findByRole('link', { name: 'Nouveau serveur' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers/new'))
    expect(await screen.findByText('Minecraft Java Edition')).toBeTruthy()
  })

  it('searches names, tags and ids, ignoring accents, and keeps the search in the URL', async () => {
    setup([prefabSummary, wordpress, valheim, enshrouded])
    const { router, user } = await renderApp('/servers/new')
    await screen.findByText('WordPress')

    await user.type(screen.getByRole('searchbox', { name: /Rechercher un prefab/ }), 'enshrouded')
    await waitFor(() => expect(shown()).toEqual(['enshrouded']))
    expect(router.currentRoute.value.query.q).toBe('enshrouded')

    await user.clear(screen.getByRole('searchbox', { name: /Rechercher un prefab/ }))
    await user.type(screen.getByRole('searchbox', { name: /Rechercher un prefab/ }), 'survival')
    await waitFor(() => expect(shown()).toEqual(['valheim', 'enshrouded']))
  })

  it('says when nothing matches and resets the filters', async () => {
    setup([prefabSummary, wordpress])
    const { user } = await renderApp('/servers/new?q=zzz')

    expect(await screen.findByText('Aucun prefab ne correspond')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Réinitialiser' }))

    await waitFor(() => expect(shown()).toHaveLength(2))
  })

  it('filters by category, with the count of each', async () => {
    setup([prefabSummary, wordpress, valheim])
    const { user } = await renderApp('/servers/new')
    const categories = await screen.findByRole('group', { name: 'Catégories' })

    expect(within(categories).getByRole('button', { name: /Jeu\s*2/ })).toBeTruthy()
    await user.click(within(categories).getByRole('button', { name: /Web\s*1/ }))

    await waitFor(() => expect(shown()).toEqual(['wordpress']))
  })

  it('sorts the most deployed first by default', async () => {
    setup([prefabSummary, wordpress, valheim])
    await renderApp('/servers/new')

    await screen.findByText('Valheim')
    expect(shown()[0]).toBe('valheim')
  })

  it('pages through a large catalog', async () => {
    const many = Array.from({ length: 30 }, (_, i) => prefab(`game-${String(i).padStart(2, '0')}`, `Jeu ${String(i).padStart(2, '0')}`))
    setup(many)
    const { router, user } = await renderApp('/servers/new?sort=name')

    await screen.findByText('Jeu 00')
    expect(shown()).toHaveLength(24)
    expect(screen.getByText('1–24 sur 30')).toBeTruthy()

    await user.click(screen.getByRole('button', { name: 'Page suivante' }))

    await waitFor(() => expect(shown()).toHaveLength(6))
    expect(shown()[0]).toBe('game-24')
    expect(router.currentRoute.value.query.page).toBe('2')
  })

  it('lists prefabs as rows when asked', async () => {
    setup([prefabSummary])
    const { user } = await renderApp('/servers/new')
    await screen.findByText('Minecraft Java Edition')

    await user.click(screen.getByRole('button', { name: 'Liste' }))

    expect(screen.getByRole('button', { name: 'Liste' }).getAttribute('aria-pressed')).toBe('true')
    expect(document.querySelector('ul.divide-y [data-prefab="minecraft-java"]')).toBeTruthy()
  })

  it('opens a prefab named in the URL and goes back to the catalog', async () => {
    setup([prefabSummary, wordpress])
    const { router, user } = await renderApp('/servers/new?prefab=minecraft-java')

    expect(await screen.findByLabelText('Nom')).toBeTruthy()
    expect(screen.getByText('minecraft-java · 1.0.0')).toBeTruthy()

    await user.click(screen.getByRole('button', { name: 'Changer de prefab' }))

    await waitFor(() => expect(router.currentRoute.value.query.prefab).toBeUndefined())
    expect(await screen.findByText('WordPress')).toBeTruthy()
  })

  it('refuses a prefab that cannot be deployed', async () => {
    setup([{ ...prefabSummary, deployable: false }])
    await renderApp('/servers/new?prefab=minecraft-java&version=1.0.0')

    expect(await screen.findByText('Prefab introuvable ou non déployable')).toBeTruthy()
  })
})
