// Contenu d'un serveur (ADR-0020) : version du jeu à figer, installés, mise à jour, activation,
// suppression avec les dépendances, recherche et installation.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const CONTENT = `${API}/servers/${SERVER_ID}/content`
const PAGE = `/servers/${SERVER_ID}/content/mods`
const SODIUM = '01a17f10-0000-7000-8000-000000000001'
const FAPI = '01a17f10-0000-7000-8000-000000000002'

const pinned: Schemas['ContentSlotDto'] = {
  slot: 'mods',
  kind: 'mods',
  loader: 'fabric',
  game_version: '1.21.4',
  blocked: null,
  latest_game_version: null,
  mode: 'download',
  upload: true,
  fields: [],
  sources: [{ provider: 'modrinth', blocked: null }],
}

/** Modrinth et CurseForge ; CurseForge sans clé d'API tant que `curseforge` ne la lève pas. */
const twoSources = (curseforge: Schemas['ContentSlotSourceDto']['blocked'] = null): Schemas['ContentSlotDto'] => ({
  ...pinned,
  sources: [
    { provider: 'modrinth', blocked: null },
    { provider: 'curseforge', blocked: curseforge },
  ],
})

function hit(overrides: Partial<Schemas['ProjectHitDto']> = {}): Schemas['ProjectHitDto'] {
  return {
    project_id: 'sodium',
    slug: 'sodium',
    title: 'Sodium',
    description: 'Fast rendering.',
    author: 'jellysquid',
    icon_url: null,
    downloads: 1_500_000,
    page_url: 'https://modrinth.com/project/sodium',
    download_allowed: true,
    ...overrides,
  }
}

function item(overrides: Partial<Schemas['ContentDto']> = {}): Schemas['ContentDto'] {
  return {
    id: SODIUM,
    name: 'Sodium',
    page_url: 'https://modrinth.com/project/sodium',
    values: {},
    source: 'modrinth',
    project_id: 'sodium',
    version_id: 'sodium-1',
    version_name: '0.6.0',
    icon_url: null,
    file_name: 'sodium-0.6.0.jar',
    size_bytes: 1_048_576,
    enabled: true,
    state: 'installed',
    failure: null,
    kind: 'file',
    modpack_id: null,
    required_by: [],
    orphans: [FAPI],
    packages: [],
    pending: false,
    update: null,
    created_at: '2026-10-27T10:00:00Z',
    ...overrides,
  }
}

const fapi = item({ id: FAPI, name: 'Fabric API', project_id: 'fabric-api', file_name: 'fabric-api.jar', required_by: [SODIUM], orphans: [] })

function setup(slot = pinned, items = [item({ update: { version_id: 'sodium-2', version_name: '0.6.1' } }), fapi]) {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ settings: { loader: 'fabric', version: '1.21.4' } }))),
    http.get(CONTENT, () => HttpResponse.json([slot])),
    http.get(`${CONTENT}/mods`, () => HttpResponse.json({ items })),
    http.get(`${API}/content/loaders`, () =>
      HttpResponse.json([{ name: 'fabric', icon_svg: '<svg viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>' }]),
    ),
  )
}

describe('server content', () => {
  it('shows the loader icon of the source next to the compatibility', async () => {
    setup()
    await renderApp(PAGE)

    const compat = await screen.findByText(/fabric/i, { selector: 'p' })
    await waitFor(() => {
      const icon = compat.querySelector<HTMLElement>('span[aria-hidden="true"]')
      expect(icon?.style.maskImage).toContain('data:image/svg+xml')
    })
    expect(compat.querySelector('svg')).toBeNull()
  })

  it('lists the slots of the prefab in the server menu', async () => {
    setup()
    await renderApp(`/servers/${SERVER_ID}`)

    await screen.findByText('Sécurité')
    const nav = screen.getAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))!
    expect(await within(nav).findByRole('link', { name: 'Mods' })).toBeTruthy()
  })

  it('offers to pin the game version before any mod', async () => {
    setup({ ...pinned, game_version: 'LATEST', blocked: 'game_version_not_pinned', latest_game_version: '1.21.4' })
    let body: unknown
    server.use(
      http.patch(`${API}/servers/${SERVER_ID}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(serverDto())
      }),
    )
    const { user } = await renderApp(PAGE)

    expect(await screen.findByText(/La version du jeu n’est pas figée/)).toBeTruthy()
    expect(screen.getByRole('tab', { name: 'Parcourir' }).hasAttribute('disabled')).toBe(true)
    await user.click(screen.getByRole('button', { name: 'Figer en 1.21.4' }))

    await waitFor(() => expect(body).toEqual({ settings: { version: '1.21.4' } }))
  })

  it('shows what each mod needs, updates and disables it', async () => {
    setup()
    const calls: string[] = []
    server.use(
      http.post(`${CONTENT}/mods/${SODIUM}/update`, () => {
        calls.push('update')
        return HttpResponse.json(item(), { status: 202 })
      }),
      http.patch(`${CONTENT}/mods/${FAPI}`, async ({ request }) => {
        calls.push(`toggle ${JSON.stringify(await request.json())}`)
        return HttpResponse.json(fapi)
      }),
    )
    const { user } = await renderApp(PAGE)

    expect(await screen.findByText('Dépendance de Sodium')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Supprimer Fabric API' }).hasAttribute('disabled')).toBe(true)
    await user.click(screen.getByRole('button', { name: 'Mettre à jour Sodium en 0.6.1' }))
    await user.click(screen.getByRole('switch', { name: 'Activer Fabric API' }))

    await waitFor(() => expect(calls).toEqual(['update', 'toggle {"enabled":false}']))
  })

  it('removes a mod with the dependencies nothing else needs', async () => {
    setup()
    let query: URLSearchParams | undefined
    server.use(
      http.delete(`${CONTENT}/mods/${SODIUM}`, ({ request }) => {
        query = new URL(request.url).searchParams
        return HttpResponse.json({ removed: [SODIUM, FAPI] })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('button', { name: 'Supprimer Sodium' }))
    const dialog = await screen.findByRole('alertdialog')
    const orphans = within(dialog).getByRole('checkbox')
    expect((orphans as HTMLInputElement).checked).toBe(false)
    expect(within(dialog).getByText(/Fabric API/)).toBeTruthy()
    await user.click(orphans)
    await user.click(within(dialog).getByRole('button', { name: 'Supprimer' }))

    await waitFor(() => expect(query?.get('with_orphans')).toBe('true'))
  })

  it('finds a mod and installs the chosen version with its dependencies', async () => {
    setup(pinned, [])
    let body: unknown
    server.use(
      http.get(`${CONTENT}/mods/search`, () =>
        HttpResponse.json({
          total: 1,
          hits: [hit()],
        }),
      ),
      http.get(`${CONTENT}/mods/projects/sodium/versions`, () =>
        HttpResponse.json([
          { id: 'sodium-2', name: 'Sodium 0.6.1', version_number: '0.6.1', published: '2026-10-20T00:00:00Z', loaders: ['fabric'], game_versions: ['1.21.4'], file_name: 'sodium-0.6.1.jar', size: 1024, required_projects: ['fabric-api'] },
        ]),
      ),
      http.post(`${CONTENT}/mods`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json([item(), fapi], { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    expect(await screen.findByText('Fast rendering.')).toBeTruthy()
    // La fiche du projet, pour savoir ce qu'il fait.
    const page = screen.getByRole('link', { name: 'Page de Sodium chez la source' })
    expect(page.getAttribute('href')).toBe('https://modrinth.com/project/sodium')
    expect(page.getAttribute('target')).toBe('_blank')
    await user.click(screen.getByRole('button', { name: 'Ajouter' }))
    const dialog = await screen.findByRole('dialog')
    expect(await within(dialog).findByText('Installé avec 1 dépendance(s) obligatoire(s) qui manqueraient.')).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Installer' }))

    await waitFor(() => expect(body).toEqual({ version_id: 'sodium-2', source: 'modrinth', values: {} }))
  })

  it('searches CurseForge when picked, and installs from it', async () => {
    setup(twoSources(), [])
    let searched: string | null = null
    let body: unknown
    server.use(
      http.get(`${CONTENT}/mods/search`, ({ request }) => {
        searched = new URL(request.url).searchParams.get('source')
        return HttpResponse.json({
          total: 1,
          hits: searched === 'curseforge' ? [hit({ project_id: '238222', slug: 'jei', title: 'JEI', description: 'Item viewer.' })] : [hit()],
        })
      }),
      http.get(`${CONTENT}/mods/projects/238222/versions`, () =>
        HttpResponse.json([
          { id: '5101', name: 'jei 19.0', version_number: 'jei 19.0', published: '2026-10-20T00:00:00Z', loaders: ['fabric'], game_versions: ['1.21.4'], file_name: 'jei.jar', size: 1024, required_projects: [] },
        ]),
      ),
      http.post(`${CONTENT}/mods`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json([item({ source: 'curseforge', project_id: '238222', name: 'JEI' })], { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    expect(await screen.findByText('Fast rendering.')).toBeTruthy()
    await user.click(screen.getByRole('radio', { name: 'CurseForge' }))
    expect(await screen.findByText('Item viewer.')).toBeTruthy()
    expect(searched).toBe('curseforge')
    await user.click(screen.getByRole('button', { name: 'Ajouter' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(await within(dialog).findByRole('button', { name: 'Installer' }))

    await waitFor(() => expect(body).toEqual({ version_id: '5101', source: 'curseforge', values: {} }))
  })

  it('sorts and filters by categories of the source, kept in the URL', async () => {
    setup(twoSources(), [])
    const searches: URLSearchParams[] = []
    server.use(
      http.get(`${CONTENT}/mods/search`, ({ request }) => {
        searches.push(new URL(request.url).searchParams)
        return HttpResponse.json({ total: 1, hits: [hit()] })
      }),
      http.get(`${CONTENT}/mods/categories`, ({ request }) =>
        HttpResponse.json(
          new URL(request.url).searchParams.get('source') === 'curseforge'
            ? [{ id: '421', name: 'Map and Information', icon_svg: null, icon_url: null }]
            : [
                { id: 'magic', name: 'Magic', icon_svg: null, icon_url: null },
                { id: 'storage', name: 'Storage', icon_svg: null, icon_url: null },
              ],
        ),
      ),
    )
    const { user, router } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    await screen.findByText('Fast rendering.')
    await user.click(screen.getByRole('combobox', { name: 'Trier' }))
    await user.click(await screen.findByRole('option', { name: 'Téléchargements' }))
    await user.click(await screen.findByRole('button', { name: /Catégories/ }))
    await user.click(await screen.findByText('Magic'))
    await user.click(screen.getByText('Storage'))

    await waitFor(() => expect(router.currentRoute.value.query).toMatchObject({ sort: 'downloads', categories: 'magic,storage' }))
    await waitFor(() => {
      const last = searches.at(-1)!
      expect(last.get('sort')).toBe('downloads')
      expect(last.get('categories')).toBe('magic,storage')
      expect(last.get('source')).toBe('modrinth')
    })

    // Les catégories sont propres à chaque source ; le tri reste.
    await user.keyboard('{Escape}')
    await user.click(screen.getByRole('radio', { name: 'CurseForge' }))
    await waitFor(() => expect(router.currentRoute.value.query).toMatchObject({ source: 'curseforge', sort: 'downloads' }))
    expect(router.currentRoute.value.query.categories).toBeUndefined()
    await waitFor(() => expect(searches.at(-1)!.get('source')).toBe('curseforge'))
    expect(searches.at(-1)!.get('categories')).toBeNull()
  })

  it('points to the settings while CurseForge has no API key', async () => {
    setup(twoSources('content_source_not_configured'), [])
    server.use(http.get(`${CONTENT}/mods/search`, () => HttpResponse.json({ total: 1, hits: [hit()] })))
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    await user.click(await screen.findByRole('radio', { name: 'CurseForge' }))

    expect(await screen.findByText('CurseForge demande une clé d’API, à régler dans les paramètres.')).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Régler la clé' }).getAttribute('href')).toBe('/settings/content-sources')
  })

  it('links to the page of a mod whose author forbids its download', async () => {
    setup(twoSources(), [])
    server.use(
      http.get(`${CONTENT}/mods/search`, () =>
        HttpResponse.json({ total: 1, hits: [hit({ title: 'Closed Mod', download_allowed: false, page_url: 'https://www.curseforge.com/minecraft/mc-mods/closed' })] }),
      ),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))

    expect(await screen.findByText('Téléchargement interdit par l’auteur')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Ajouter' })).toBeNull()
    expect(screen.getByRole('link', { name: 'Voir la page' }).getAttribute('href')).toBe('https://www.curseforge.com/minecraft/mc-mods/closed')
  })

  it('names a source Kuro does not support yet, without the Minecraft loader advice', async () => {
    setup(
      {
        ...pinned,
        mode: 'reference',
        upload: false,
        loader: null,
        game_version: null,
        blocked: 'content_source_unsupported',
        sources: [{ provider: 'steam-workshop', blocked: 'content_source_unsupported' }],
      },
      [],
    )
    await renderApp(PAGE)

    expect(await screen.findByText(/Ce contenu vient de Steam Workshop, une source que Kuro ne gère pas encore/)).toBeTruthy()
    expect(screen.queryByText(/choisissez un loader/)).toBeNull()
    expect(screen.getByRole('tab', { name: 'Parcourir' }).hasAttribute('disabled') || screen.getByRole('tab', { name: 'Parcourir' }).getAttribute('data-disabled') !== null).toBe(true)
    expect(screen.queryByRole('button', { name: 'Déposer un fichier' })).toBeNull()
  })

  it('asks the values a referenced mod needs, then edits them', async () => {
    const workshop: Schemas['ContentSlotDto'] = {
      ...pinned,
      mode: 'reference',
      upload: false,
      loader: null,
      game_version: null,
      fields: [{ key: 'mod_ids', type: 'list', required: false, prefill: true }],
      sources: [{ provider: 'curseforge', blocked: null }],
    }
    const installed = item({ id: SODIUM, name: 'Hydrocraft', kind: 'reference', source: 'curseforge', values: { mod_ids: ['Hydrocraft'] } })
    setup(workshop, [installed])
    let added: unknown
    let edited: unknown
    server.use(
      http.get(`${CONTENT}/mods/search`, () => HttpResponse.json({ total: 1, hits: [hit({ project_id: '500', title: 'Better Sorting', description: 'Sorted.' })] })),
      http.get(`${CONTENT}/mods/projects/500/versions`, () =>
        HttpResponse.json([{ id: '5001', name: 'v1', version_number: 'v1', published: '2026-10-20T00:00:00Z', loaders: [], game_versions: [], file_name: null, size: null, required_projects: [] }]),
      ),
      http.get(`${CONTENT}/mods/projects/500`, () => HttpResponse.json({ project_id: '500', title: 'Better Sorting', icon_url: null, prefill: {} })),
      http.post(`${CONTENT}/mods`, async ({ request }) => {
        added = await request.json()
        return HttpResponse.json([item({ id: FAPI, name: 'Better Sorting', kind: 'reference' })], { status: 202 })
      }),
      http.put(`${CONTENT}/mods/${SODIUM}/values`, async ({ request }) => {
        edited = await request.json()
        return HttpResponse.json(installed)
      }),
    )
    const { user } = await renderApp(PAGE)

    // Valeurs listées sous le contenu installé.
    expect(await screen.findByText('Hydrocraft', { selector: 'span.font-mono' })).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Modifier les valeurs de Hydrocraft' }))
    const editDialog = await screen.findByRole('dialog')
    const field = within(editDialog).getByRole('textbox')
    await user.clear(field)
    await user.type(field, 'Hydrocraft{Enter}HydrocraftFood')
    await user.click(within(editDialog).getByRole('button', { name: 'Enregistrer' }))
    await waitFor(() => expect(edited).toEqual({ values: { mod_ids: ['Hydrocraft', 'HydrocraftFood'] } }))

    await user.click(screen.getByRole('tab', { name: 'Parcourir' }))
    await user.click(await screen.findByRole('button', { name: 'Ajouter' }))
    const addDialog = await screen.findByRole('dialog')
    // Rien dans la description : la page du projet donne les identifiants.
    expect(await within(addDialog).findByText(/Rien trouvé dans la description du projet/)).toBeTruthy()
    expect(within(addDialog).getByRole('link', { name: 'Voir la page' }).getAttribute('href')).toBe('https://modrinth.com/project/sodium')
    await user.type(await within(addDialog).findByRole('textbox'), 'BetterSorting')
    await user.click(within(addDialog).getByRole('button', { name: 'Installer' }))
    await waitFor(() => expect(added).toEqual({ version_id: '5001', source: 'curseforge', values: { mod_ids: ['BetterSorting'] } }))
  })

  it('pre-fills the mod ids a Workshop item gives in its description', async () => {
    setup(
      {
        ...pinned,
        mode: 'reference',
        upload: false,
        loader: null,
        game_version: null,
        fields: [{ key: 'mod_ids', type: 'list', required: false, prefill: true }],
        sources: [{ provider: 'steam-workshop', blocked: null }],
      },
      [],
    )
    let added: unknown
    server.use(
      http.get(`${CONTENT}/mods/search`, ({ request }) => {
        expect(new URL(request.url).searchParams.get('source')).toBe('steam-workshop')
        return HttpResponse.json({ total: 1, hits: [hit({ project_id: '2001', title: 'Hydrocraft', author: '', page_url: null })] })
      }),
      http.get(`${CONTENT}/mods/projects/2001/versions`, () =>
        HttpResponse.json([{ id: '2001', name: 'Hydrocraft', version_number: '2026-10-20', published: '2026-10-20T00:00:00Z', loaders: [], game_versions: [], file_name: '2001', size: null, required_projects: ['3001'] }]),
      ),
      http.get(`${CONTENT}/mods/projects/2001`, () =>
        HttpResponse.json({ project_id: '2001', title: 'Hydrocraft', icon_url: null, prefill: { mod_ids: ['Hydrocraft', 'HydrocraftFood'] } }),
      ),
      http.post(`${CONTENT}/mods`, async ({ request }) => {
        added = await request.json()
        return HttpResponse.json([item({ name: 'Hydrocraft', kind: 'reference', source: 'steam-workshop' })], { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    expect(await screen.findByText('Hydrocraft')).toBeTruthy()
    // Le Workshop ne donne pas d'auteur : pas de « par » vide.
    expect(screen.queryByText(/^par/)).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Ajouter' }))
    const dialog = await screen.findByRole('dialog')
    expect(await within(dialog).findByText('Valeurs reprises de la description du projet : vérifiez-les avant d’ajouter.')).toBeTruthy()
    const field = within(dialog).getByRole('textbox')
    await waitFor(() => expect(field).toHaveProperty('value', 'Hydrocraft\nHydrocraftFood'))
    expect(within(dialog).queryByText('2001', { selector: 'p.font-mono' })).toBeNull()

    // La saisie reste à l'administrateur : il retire ce que le serveur ne doit pas charger.
    await user.clear(field)
    await user.type(field, 'Hydrocraft')
    await user.click(within(dialog).getByRole('button', { name: 'Installer' }))
    await waitFor(() => expect(added).toEqual({ version_id: '2001', source: 'steam-workshop', values: { mod_ids: ['Hydrocraft'] } }))
  })

  it('lists the packs an add-on was split into', async () => {
    const addon = item({
      name: 'More Mobs',
      source: 'curseforge',
      file_name: 'more-mobs.mcaddon',
      orphans: [],
      packages: [
        { dir: '/data/behavior_packs', name: 'more-mobs-1', title: 'More Mobs', version: '1.2.0' },
        { dir: '/data/resource_packs', name: 'more-mobs-2', title: null, version: null },
      ],
    })
    setup({ ...pinned, loader: null, game_version: null, sources: [{ provider: 'curseforge', blocked: null }] }, [addon])
    await renderApp(PAGE)

    const packs = await screen.findByRole('list', { name: 'Packs de l’archive' })
    const items = within(packs).getAllByRole('listitem').map((li) => li.textContent?.replace(/\s+/g, ' ').trim())
    expect(items).toEqual([
      'More Mobs 1.2.0, rangé dans /data/behavior_packs/more-mobs-1',
      'more-mobs-2 —, rangé dans /data/resource_packs/more-mobs-2',
    ])
  })

  it('sends one install for a double click', async () => {
    setup({ ...pinned, sources: [{ provider: 'modrinth', blocked: null }] }, [])
    let installs = 0
    server.use(
      http.get(`${CONTENT}/mods/search`, () => HttpResponse.json({ total: 1, hits: [hit()] })),
      http.get(`${CONTENT}/mods/projects/sodium/versions`, () =>
        HttpResponse.json([{ id: 'sodium-1', name: '0.6.0', version_number: '0.6.0', published: '2026-10-20T00:00:00Z', loaders: ['fabric'], game_versions: ['1.21.4'], file_name: 'sodium.jar', size: 1, required_projects: [] }]),
      ),
      http.post(`${CONTENT}/mods`, async () => {
        installs += 1
        await new Promise((resolve) => setTimeout(resolve, 50))
        return HttpResponse.json([item()], { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    await user.click(await screen.findByRole('button', { name: 'Ajouter' }))
    const dialog = await screen.findByRole('dialog')
    const button = await within(dialog).findByRole('button', { name: 'Installer' })
    await user.dblClick(button)

    await waitFor(() => expect(installs).toBe(1))
    await new Promise((resolve) => setTimeout(resolve, 100))
    expect(installs).toBe(1)
  })

  it('says what waits for the restart of a running server, and restarts it', async () => {
    setup(pinned, [item({ pending: true }), fapi])
    let restarted: unknown
    server.use(
      http.get(`${API}/servers/${SERVER_ID}`, () =>
        HttpResponse.json(
          serverDto({ state: 'running', restart_required: true, pending_changes: { contents: 1, removals: 2 }, settings: { loader: 'fabric', version: '1.21.4' } }),
        ),
      ),
      http.post(`${API}/servers/${SERVER_ID}/power`, async ({ request }) => {
        restarted = await request.json()
        return new HttpResponse(null, { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    const banner = await screen.findByRole('status')
    expect(banner.textContent).toContain('Changements en attente')
    expect(banner.textContent).toContain('1 contenu ajouté ou modifié, 2 contenus retirés : appliqués au prochain redémarrage du serveur.')
    // Le contenu concerné le dit aussi.
    expect(await screen.findByText('Au prochain redémarrage')).toBeTruthy()

    await user.click(within(banner).getByRole('button', { name: 'Redémarrer' }))
    await waitFor(() => expect(restarted).toEqual({ action: 'restart' }))
  })

  it('tells that a mod added to a running server waits for its restart', async () => {
    setup(pinned, [])
    server.use(
      http.get(`${API}/servers/${SERVER_ID}`, () =>
        HttpResponse.json(serverDto({ state: 'running', settings: { loader: 'fabric', version: '1.21.4' } })),
      ),
      http.get(`${CONTENT}/mods/search`, () => HttpResponse.json({ total: 1, hits: [hit()] })),
      http.get(`${CONTENT}/mods/projects/sodium/versions`, () =>
        HttpResponse.json([{ id: 'sodium-1', name: '0.6.0', version_number: '0.6.0', published: '2026-10-20T00:00:00Z', loaders: ['fabric'], game_versions: ['1.21.4'], file_name: 'sodium.jar', size: 1, required_projects: [] }]),
      ),
      http.post(`${CONTENT}/mods`, () => HttpResponse.json([item()], { status: 202 })),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Parcourir' }))
    await user.click(await screen.findByRole('button', { name: 'Ajouter' }))
    await user.click(await within(await screen.findByRole('dialog')).findByRole('button', { name: 'Installer' }))

    expect(await screen.findByText('Sodium ajouté.')).toBeTruthy()
    expect(screen.getByText('Le serveur tourne : ce changement sera appliqué à son prochain redémarrage.')).toBeTruthy()
  })

  it('explains that the game downloads referenced mods itself', async () => {
    setup({ ...pinned, mode: 'reference', loader: null, game_version: null, sources: [{ provider: 'curseforge', blocked: null }] }, [])
    await renderApp(PAGE)

    expect(await screen.findByText(/Le jeu télécharge lui-même ces mods/)).toBeTruthy()
    expect(screen.queryByRole('tab', { name: 'Modpacks' })).toBeNull()
  })

  it('explains a vanilla server cannot load mods', async () => {
    setup({ ...pinned, loader: 'vanilla', blocked: 'content_not_supported' }, [])
    await renderApp(PAGE)

    expect(await screen.findByText(/choisissez un loader/)).toBeTruthy()
  })
})

describe('modpacks and uploads', () => {
  it('drops a jar from the computer into the slot', async () => {
    setup(pinned, [])
    let fileName: string | null = null
    let size = 0
    server.use(
      http.post(`${CONTENT}/mods/upload`, async ({ request }) => {
        fileName = new URL(request.url).searchParams.get('file_name')
        size = (await request.arrayBuffer()).byteLength
        return HttpResponse.json(item({ id: 'u1', name: 'mine', source: 'upload', file_name: 'mine.jar', orphans: [] }), { status: 201 })
      }),
    )
    const { user } = await renderApp(PAGE)

    const input = await screen.findByLabelText('Déposer un fichier', { selector: 'input' })
    await user.upload(input as HTMLInputElement, new File(['PK jar bytes'], 'mine.jar'))

    await waitFor(() => expect(fileName).toBe('mine.jar'))
    expect(size).toBe(12)
    expect(await screen.findByText('mine.jar déposé.')).toBeTruthy()
  })

  it('lists the files of a modpack under it, without their own actions', async () => {
    const pack = item({ id: 'p1', name: 'Fabulously Optimized', kind: 'modpack', file_name: 'fo.mrpack', orphans: [], update: null })
    const child = item({ id: 'c1', name: 'lithium', source: 'modpack', file_name: 'mods/lithium.jar', modpack_id: 'p1', orphans: [], update: null })
    setup(pinned, [pack, child])
    await renderApp(PAGE)

    expect(await screen.findByText('Modpack · 1 fichiers')).toBeTruthy()
    expect(screen.queryByRole('switch', { name: 'Activer lithium' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Supprimer lithium' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Supprimer Fabulously Optimized' })).toBeTruthy()
    expect(screen.getByText('mods/lithium.jar')).toBeTruthy()
  })

  it('installs a modpack after showing the version and loader it sets', async () => {
    setup(pinned, [])
    let body: unknown
    server.use(
      http.get(`${CONTENT}/mods/modpacks/search`, () =>
        HttpResponse.json({ total: 1, hits: [{ project_id: 'fo', slug: 'fo', title: 'Fabulously Optimized', description: 'Pack.', author: 'robotkoer', icon_url: null, downloads: 9000 }] }),
      ),
      http.get(`${CONTENT}/mods/modpacks/fo/versions`, () =>
        HttpResponse.json([{ id: 'fo-1', name: 'FO 6', version_number: '6.0', published: '2026-10-01T00:00:00Z', loaders: ['fabric'], game_versions: ['1.21.4'], file_name: 'fo.mrpack', size: 1024, required_projects: [] }]),
      ),
      http.post(`${CONTENT}/mods/modpacks`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json([], { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('tab', { name: 'Modpacks' }))
    await user.click(await screen.findByRole('button', { name: 'Ajouter' }))
    const dialog = await screen.findByRole('dialog')
    expect(await within(dialog).findByText(/règle la version du jeu \(1\.21\.4\) et le loader \(fabric\)/)).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Installer' }))

    await waitFor(() => expect(body).toEqual({ version_id: 'fo-1' }))
  })
})
