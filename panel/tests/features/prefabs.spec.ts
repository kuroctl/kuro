// Gestion des prefabs : section du rail, catalogue (une carte par prefab, versions, serveurs,
// origine ; recherche, filtres, vue en liste), import d'une archive et refus détaillé, détail et suppression d'une version.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, localeFr, manifest, prefabSummary, SERVER_ID } from '../support/servers'

const v2: Schemas['PrefabSummaryDto'] = { ...prefabSummary, version: '1.1.0', source: 'uploaded', servers: 0, imported_at: '2026-10-20T00:00:00Z' }
const v1: Schemas['PrefabSummaryDto'] = { ...prefabSummary, servers: 2 }

function setup(list = [v1, v2]) {
  server.use(
    ...signedIn(),
    http.get(`${API}/prefabs`, () => HttpResponse.json(list)),
    http.get(`${API}/prefabs/minecraft-java/:version`, ({ params }) =>
      HttpResponse.json({ ...list.find((p) => p.version === params.version)!, manifest: { ...manifest, version: params.version }, locales: ['fr'] }),
    ),
    http.get(`${API}/prefabs/minecraft-java/:version/locales/fr`, () => HttpResponse.json(localeFr)),
    http.get(`${API}/prefabs/minecraft-java/:version/servers`, ({ params }) =>
      HttpResponse.json(params.version === '1.0.0' ? [{ id: SERVER_ID, name: 'Survie', state: 'running' }] : []),
    ),
    ...catalog(),
  )
}

describe('prefabs', () => {
  it('says a draft version deploys on a test node only, and why', async () => {
    setup([{ ...v1, test_nodes_only: true, draft_reason: 'not yet tried on a real node' }])
    await renderApp('/prefabs')

    const card = (await screen.findByText('Minecraft Java Edition')).closest<HTMLElement>('[data-prefab]')!
    expect(within(card).getByLabelText('Brouillon · nœud de test : Brouillon : not yet tried on a real node')).toBeTruthy()
  })

  it('says when a game needs more than a project ceiling', async () => {
    setup([{ ...v1, deployable: false, over_ceiling: ['memory_mb'] }])
    await renderApp('/prefabs')

    const card = (await screen.findByText('Minecraft Java Edition')).closest<HTMLElement>('[data-prefab]')!
    expect(within(card).getByLabelText(/Ce jeu demande plus que le plafond du projet \(Mémoire \(Mo\)\)/)).toBeTruthy()
  })

  it('says when a game waits for the Steam account of installations', async () => {
    setup([{ ...v1, deployable: false, steam_account_missing: true }])
    await renderApp('/prefabs')

    const card = (await screen.findByText('Minecraft Java Edition')).closest<HTMLElement>('[data-prefab]')!
    expect(within(card).getByLabelText(/Ce jeu s’installe avec un compte Steam : réglez-le dans Paramètres › Compte Steam/)).toBeTruthy()
  })

  it('has its own section with one card per prefab', async () => {
    setup()
    await renderApp('/prefabs')

    const card = (await screen.findByText('Minecraft Java Edition')).closest<HTMLElement>('[data-prefab]')!
    expect(within(card).getByText('minecraft-java · 1.1.0')).toBeTruthy()
    expect(within(card).getByText('2 versions')).toBeTruthy()
    expect(within(card).getByText('2 serveurs')).toBeTruthy()
    expect(within(card).getByText('Importé')).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Prefabs' })).toBeTruthy()
  })

  it('shows the portrait thumbnail on the card when the prefab has one', async () => {
    setup([v1, { ...v2, assets: ['thumbnail'] }])
    server.use(
      http.get(`${API}/prefabs/minecraft-java/1.1.0/assets/thumbnail`, () =>
        HttpResponse.arrayBuffer(new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer, { headers: { 'Content-Type': 'image/png' } }),
      ),
    )
    await renderApp('/prefabs')

    const card = (await screen.findByText('Minecraft Java Edition')).closest<HTMLElement>('[data-prefab]')!
    await waitFor(() => expect(card.querySelector('[data-slot="prefab-thumbnail"]')?.getAttribute('src')).toMatch(/^blob:/))
  })

  it('reloads the official prefabs and says what was added', async () => {
    setup()
    let called = false
    server.use(
      http.post(`${API}/prefabs/reload`, () => {
        called = true
        return HttpResponse.json({ imported: [], unchanged: ['minecraft-java@1.1.0'], assets_added: ['terraria@1.0.0: icon, cover'], conflicts: [], invalid: [] })
      }),
    )
    const { user } = await renderApp('/prefabs')

    await user.click(await screen.findByRole('button', { name: 'Recharger' }))

    await waitFor(() => expect(called).toBe(true))
    expect(await screen.findByText('Prefabs rechargés : 0 nouvelle(s) version(s), 1 image(s) ajoutée(s).')).toBeTruthy()
  })

  it('imports an archive and opens the new version', async () => {
    setup()
    let size = 0
    let type: string | null = null
    server.use(
      http.post(`${API}/prefabs/import`, async ({ request }) => {
        type = request.headers.get('content-type')
        size = (await request.arrayBuffer()).byteLength
        return HttpResponse.json({ ...v2, manifest, locales: ['fr'] }, { status: 201 })
      }),
    )
    const { user, router } = await renderApp('/prefabs')

    const input = await screen.findByLabelText('Importer un .kprefab', { selector: 'input' })
    await user.upload(input as HTMLInputElement, new File(['PK archive'], 'mc.kprefab'))

    await waitFor(() => expect(router.currentRoute.value.fullPath).toBe('/prefabs/minecraft-java?version=1.1.0'))
    expect(size).toBe(10)
    expect(type).toBe('application/zip')
    expect(await screen.findByText('Minecraft Java Edition 1.1.0 importé.')).toBeTruthy()
  })

  it('details why an archive was refused', async () => {
    setup()
    server.use(
      http.post(`${API}/prefabs/import`, () =>
        problem(422, 'invalid_prefab', {
          params: { reason: 'invalid', issues: [{ path: 'services.server.image', message: 'must pin a tag' }] },
        }),
      ),
    )
    const { user } = await renderApp('/prefabs')

    await user.upload(
      (await screen.findByLabelText('Importer un .kprefab', { selector: 'input' })) as HTMLInputElement,
      new File(['PK'], 'bad.kprefab'),
    )

    const alert = await screen.findByRole('alert')
    expect(within(alert).getByText('Archive refusée')).toBeTruthy()
    expect(within(alert).getByText('services.server.image')).toBeTruthy()
  })

  it('shows a version and deletes it only when no server uses it', async () => {
    setup()
    let deleted: string | undefined
    server.use(
      http.delete(`${API}/prefabs/minecraft-java/:version`, ({ params }) => {
        deleted = String(params.version)
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp('/prefabs/minecraft-java?version=1.0.0')

    expect(await screen.findByText('docker.io/itzg/minecraft-server:2026.10.1-java25')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Supprimer cette version' }).hasAttribute('disabled')).toBe(true)
    await user.click(screen.getByRole('tab', { name: 'Serveurs' }))
    expect(await screen.findByRole('link', { name: /Survie/ })).toBeTruthy()

    await user.click(screen.getByRole('combobox', { name: 'Version' }))
    await user.click(await screen.findByRole('option', { name: '1.1.0' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Supprimer cette version' }).hasAttribute('disabled')).toBe(false))
    await user.click(screen.getByRole('button', { name: 'Supprimer cette version' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: 'Supprimer cette version' }))

    await waitFor(() => expect(deleted).toBe('1.1.0'))
  })

  it('exports a version as a .kprefab archive', async () => {
    setup()
    let exported: string | undefined
    server.use(
      http.get(`${API}/prefabs/minecraft-java/:version/export`, ({ params }) => {
        exported = String(params.version)
        return new HttpResponse(new Blob(['PK']), { headers: { 'Content-Type': 'application/zip' } })
      }),
    )
    let downloaded: string | undefined
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      downloaded = this.download
    })
    const { user } = await renderApp('/prefabs/minecraft-java?version=1.1.0')
    await screen.findByText('docker.io/itzg/minecraft-server:2026.10.1-java25')

    await user.click(screen.getByRole('button', { name: 'Exporter' }))

    await waitFor(() => expect(downloaded).toBe('minecraft-java-1.1.0.kprefab'))
    expect(exported).toBe('1.1.0')
    click.mockRestore()
  })

  it('optimizes the images of a version and warns about the ones too small', async () => {
    setup([v1, { ...v2, assets: ['icon', 'thumbnail'] }])
    let optimized: string | undefined
    server.use(
      http.post(`${API}/prefabs/minecraft-java/:version/assets/optimize`, ({ params }) => {
        optimized = String(params.version)
        return HttpResponse.json({
          optimized: [
            { kind: 'icon', before: { width: 1000, height: 1000, bytes: 307200 }, after: { width: 64, height: 64, bytes: 102400 }, small: false },
            { kind: 'thumbnail', before: { width: 364, height: 485, bytes: 1024 }, after: { width: 363, height: 484, bytes: 1024 }, small: true },
          ],
          unchanged: [],
          failed: [],
        })
      }),
    )
    const { user } = await renderApp('/prefabs/minecraft-java?version=1.1.0')

    await user.click(await screen.findByRole('button', { name: 'Optimiser les images' }))

    expect(await screen.findByText('2 images optimisées (200 KiB gagnés)')).toBeTruthy()
    expect(await screen.findByText('Plus petites que la convention, à remplacer par une image plus grande : vignette')).toBeTruthy()
    expect(optimized).toBe('1.1.0')
  })

  it('offers neither import nor deletion without the manage permission', async () => {
    setup([v1, { ...v2, assets: ['icon'] }])
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['prefabs.read', 'servers.read'] }),
      ),
    )
    await renderApp('/prefabs/minecraft-java?version=1.1.0')

    expect(await screen.findByText('docker.io/itzg/minecraft-server:2026.10.1-java25')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Supprimer cette version' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Optimiser les images' })).toBeNull()
    expect(screen.queryByLabelText('Importer un .kprefab')).toBeNull()
  })

  it('searches the catalog and filters it by origin', async () => {
    const wordpress: Schemas['PrefabSummaryDto'] = {
      ...prefabSummary,
      id: 'wordpress',
      category: 'web',
      tags: ['cms'],
      names: { fr: 'WordPress' },
      source: 'uploaded',
    }
    setup([v1, wordpress])
    const { router, user } = await renderApp('/prefabs')
    await screen.findByText('WordPress')

    await user.type(screen.getByRole('searchbox', { name: /Rechercher un prefab/ }), 'cms')
    await waitFor(() => expect(screen.queryByText('Minecraft Java Edition')).toBeNull())
    expect(router.currentRoute.value.query.q).toBe('cms')

    await user.clear(screen.getByRole('searchbox', { name: /Rechercher un prefab/ }))
    await user.click(await screen.findByRole('button', { name: 'Origine' }))
    await user.click(within(await screen.findByRole('dialog')).getByText('Officiel'))
    await waitFor(() => expect(screen.queryByText('WordPress')).toBeNull())
    expect(screen.getByText('Minecraft Java Edition')).toBeTruthy()
  })

  it('lists prefabs as rows that open their page', async () => {
    setup()
    const { router, user } = await renderApp('/prefabs')
    await screen.findByText('Minecraft Java Edition')

    await user.click(screen.getByRole('button', { name: 'Liste' }))
    const row = document.querySelector<HTMLElement>('ul.divide-y [data-prefab="minecraft-java"] a')!
    expect(within(row).getByText('2 versions')).toBeTruthy()
    await user.click(row)

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/prefabs/minecraft-java'))
  })
})
