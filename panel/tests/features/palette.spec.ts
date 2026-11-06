// Palette de commandes (ADR-0064) : Ctrl+K, recherche de pages, serveurs et nœuds, clavier,
// derniers objets ouverts.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { beforeEach, describe, expect, it } from 'vitest'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, NODE_ID, SERVER_ID, serverDto } from '../support/servers'

function setup() {
  server.use(
    ...signedIn(),
    http.get(`${API}/nodes`, () => HttpResponse.json([node])),
    ...catalog(),
    http.get(`${API}/servers`, () => HttpResponse.json([serverDto({ name: 'Survie' })])),
  )
}

const options = () => within(screen.getByRole('listbox')).getAllByRole('option').map((option) => option.textContent ?? '')

describe('command palette', () => {
  beforeEach(() => localStorage.clear())

  it('opens with Ctrl+K and goes to a server with the keyboard', async () => {
    setup()
    const { user, router } = await renderApp('/servers/list')

    await user.keyboard('{Control>}k{/Control}')
    const input = await screen.findByRole('combobox', { name: 'Aller à…' })
    await user.type(input, 'surv')
    await waitFor(() => expect(options()[0]).toContain('Survie'))
    await user.keyboard('{Enter}')

    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/servers/${SERVER_ID}`))
    expect(screen.queryByRole('combobox', { name: 'Aller à…' })).toBeNull()
  })

  it('finds pages without accents and moves with the arrows', async () => {
    setup()
    const { user, router } = await renderApp('/servers/list')

    await user.click(await screen.findByRole('button', { name: 'Recherche rapide' }))
    await user.type(await screen.findByRole('combobox'), 'regi')
    await waitFor(() => expect(options().some((text) => text.includes('Régions'))).toBe(true))
    const index = options().findIndex((text) => text.includes('Régions'))
    await user.keyboard(`${'{ArrowDown}'.repeat(index)}{Enter}`)

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers/regions'))
  })

  it('offers the last opened server and node first', async () => {
    setup()
    const { user, router } = await renderApp(`/servers/nodes/${NODE_ID}`)
    await router.push(`/servers/${SERVER_ID}`)

    await user.keyboard('{Control>}k{/Control}')
    await screen.findByRole('listbox')
    await waitFor(() => expect(options().slice(0, 2)).toEqual([expect.stringContaining('Survie'), expect.stringContaining(node.name)]))
  })
})
