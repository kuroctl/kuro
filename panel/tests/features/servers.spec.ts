// Serveurs : liste, création depuis un prefab, réglages générés, actions.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, NODE_ID, prefabSummary, SERVER_ID, serverDto } from '../support/servers'

describe('servers', () => {
  it('lists servers with their state and address', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers`, () => HttpResponse.json([serverDto()])))
    await renderApp('/servers/list')

    expect(await screen.findByText('Survie')).toBeTruthy()
    expect(screen.getByText('Arrêté')).toBeTruthy()
    expect(screen.getByText('0.0.0.0:25565')).toBeTruthy()
  })

  it('marks a server whose content waits for a restart', async () => {
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([serverDto({ state: 'running', restart_required: true })])),
    )
    await renderApp('/servers/list')

    expect(await screen.findByText('Redémarrage requis')).toBeTruthy()
  })

  it('proposes the resources a game recommends and shows its minimum', async () => {
    const needs = { memory_mb: { min: 3072, recommended: 4096 } }
    server.use(
      ...signedIn(),
      http.get(`${API}/prefabs`, () => HttpResponse.json([{ ...prefabSummary, resources: needs }])),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
    )
    const { user } = await renderApp('/servers')

    await user.click(await screen.findByRole('link', { name: 'Nouveau serveur' }))
    await user.click(await screen.findByText('Minecraft Java Edition'))

    const memory = (await screen.findByLabelText('Mémoire (Mo)')) as HTMLInputElement
    expect(memory.value.replace(/\D/g, '')).toBe('4096')
    expect(screen.getByText(/Ce jeu : 3.072 minimum, 4.096 recommandé\./)).toBeTruthy()
  })

  it('offers only test nodes for a draft version', async () => {
    const testNode = { ...node, id: '01a17a00-0000-7000-8000-000000000002', name: 'test-1', display_name: 'Test 1', trials: true }
    server.use(
      ...signedIn(),
      http.get(`${API}/prefabs`, () => HttpResponse.json([{ ...prefabSummary, test_nodes_only: true, draft_reason: 'new runtime image' }])),
      http.get(`${API}/nodes`, () => HttpResponse.json([node, testNode])),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
    )
    const { user } = await renderApp('/servers')

    await user.click(await screen.findByRole('link', { name: 'Nouveau serveur' }))
    await user.click(await screen.findByText('Minecraft Java Edition'))

    expect(await screen.findByText('Version en brouillon, déployable sur un nœud de test seulement : new runtime image')).toBeTruthy()
    await user.click(screen.getByRole('combobox', { name: 'Nœud' }))
    expect(await screen.findByRole('option', { name: 'Test 1' })).toBeTruthy()
    expect(screen.queryByRole('option', { name: 'Dev 1' })).toBeNull()
  })

  it('says how to get a test node when a draft has none', async () => {
    server.use(
      ...signedIn(),
      http.get(`${API}/prefabs`, () => HttpResponse.json([{ ...prefabSummary, test_nodes_only: true, draft_reason: 'x' }])),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
    )
    const { user } = await renderApp('/servers')

    await user.click(await screen.findByRole('link', { name: 'Nouveau serveur' }))
    await user.click(await screen.findByText('Minecraft Java Edition'))

    expect(await screen.findByText(/Aucun nœud de test en ligne\. Marquez-en un comme « Nœud de test »/)).toBeTruthy()
  })

  it('warns when a server is below the minimum of its game', async () => {
    server.use(
      ...signedIn(),
      http.get(`${API}/prefabs`, () => HttpResponse.json([{ ...prefabSummary, resources: { memory_mb: { min: 3072, recommended: null } } }])),
      ...catalog(),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ memory_mb: 2048 }))),
    )
    await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByText(/Sous le minimum de ce jeu \(Mémoire \(Mo\)\)/)).toBeTruthy()
  })

  it('shows the prefab icon, fetched with the session', async () => {
    let authorization: string | null = null
    server.use(
      ...signedIn(),
      http.get(`${API}/prefabs`, () => HttpResponse.json([{ ...prefabSummary, assets: ['icon'] }])),
      http.get(`${API}/prefabs/minecraft-java/1.0.0/assets/icon`, ({ request }) => {
        authorization = request.headers.get('authorization')
        return new HttpResponse(new Uint8Array([0x89, 0x50, 0x4e, 0x47]), { headers: { 'Content-Type': 'image/png' } })
      }),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([serverDto()])),
    )
    await renderApp('/servers/list')

    await screen.findByText('Survie')
    await waitFor(() => expect(document.querySelector('img[src^="blob:"]')).toBeTruthy())
    expect(authorization).toMatch(/^Bearer /)
  })

  it('falls back to the prefab initial without an icon', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers`, () => HttpResponse.json([serverDto()])))
    await renderApp('/servers/list')

    await screen.findByText('Survie')
    await waitFor(() => expect(screen.getByText('M')).toBeTruthy())
    expect(document.querySelector('img')).toBeNull()
  })

  it('invites to deploy a first server', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers`, () => HttpResponse.json([])))
    await renderApp('/servers/list')

    expect(await screen.findByText('Aucun serveur')).toBeTruthy()
  })

  it('deploys a server from a prefab in three steps', async () => {
    let body: Record<string, unknown> | undefined
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
      http.post(`${API}/servers`, async ({ request }) => {
        body = (await request.json()) as Record<string, unknown>
        return HttpResponse.json(serverDto({ state: 'deploying' }), { status: 201 })
      }),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'deploying' }))),
    )
    const { router, user } = await renderApp('/servers')

    await user.click(await screen.findByRole('link', { name: 'Nouveau serveur' }))
    await user.click(await screen.findByText('Minecraft Java Edition'))
    await user.type(await screen.findByLabelText('Nom'), 'Survie')
    await user.click(screen.getByRole('combobox', { name: 'Nœud' }))
    await user.click(await screen.findByRole('option', { name: 'Dev 1' }))
    await user.click(screen.getByRole('button', { name: 'Continuer' }))

    const motd = await screen.findByLabelText("Message d'accueil")
    expect(screen.getByText('Affiché dans la liste des serveurs.')).toBeTruthy()
    await user.clear(motd)
    await user.type(motd, 'Bienvenue')
    await user.click(screen.getByRole('button', { name: 'Déployer' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/servers/${SERVER_ID}`))
    expect(body).toMatchObject({
      name: 'Survie',
      node_id: NODE_ID,
      prefab_id: 'minecraft-java',
      prefab_version: '1.0.0',
      memory_mb: 2048,
      cpu_percent: 200,
      disk_mb: 10240,
      settings: { motd: 'Bienvenue', max_players: 20, difficulty: 'normal', loader: 'vanilla' },
      start_when_installed: true,
    })
  })

  it('can deploy without starting once installed', async () => {
    let body: Record<string, unknown> | undefined
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
      http.post(`${API}/servers`, async ({ request }) => {
        body = (await request.json()) as Record<string, unknown>
        return HttpResponse.json(serverDto({ state: 'deploying' }), { status: 201 })
      }),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'deploying' }))),
    )
    const { user } = await renderApp('/servers')

    await user.click(await screen.findByRole('link', { name: 'Nouveau serveur' }))
    await user.click(await screen.findByText('Minecraft Java Edition'))
    await user.type(await screen.findByLabelText('Nom'), 'Survie')
    await user.click(screen.getByRole('combobox', { name: 'Nœud' }))
    await user.click(await screen.findByRole('option', { name: 'Dev 1' }))
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    const start = await screen.findByRole('switch', { name: 'Démarrer une fois installé' })
    expect(start.getAttribute('aria-checked')).toBe('true')
    await user.click(start)
    await user.click(screen.getByRole('button', { name: 'Déployer' }))

    await waitFor(() => expect(body).toMatchObject({ start_when_installed: false }))
  })

  it('generates the settings sub-pages from the prefab, in its order', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())))
    await renderApp(`/servers/${SERVER_ID}`)

    await screen.findByText('Sécurité')
    const nav = screen.getAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))!
    const entries = within(nav)
      .getAllByRole('link')
      .map((link) => link.textContent?.trim())
    expect(entries).toEqual(['Vue d’ensemble', 'Console', 'Statistiques', 'Fichiers', 'Sauvegardes', 'Planification', 'Notifications', 'Ressources', 'Serveur', 'Version et loader', 'Sécurité'])
    expect(within(nav).getByText('Réglages généraux')).toBeTruthy()
    expect(within(nav).getByText('Réglages avancés')).toBeTruthy()
  })

  it('folds a settings group and filters the server menu', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())))
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    await screen.findByText('Sécurité')
    const nav = screen.getAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))!
    await user.click(within(nav).getByRole('button', { name: 'Réglages généraux' }))
    await waitFor(() => expect(within(nav).queryByRole('link', { name: 'Version et loader' })).toBeNull())

    await user.type(screen.getByRole('searchbox', { name: 'Rechercher' }), 'securite')
    const links = within(nav)
      .getAllByRole('link')
      .map((link) => link.textContent?.trim())
    expect(links).toEqual(['Sécurité'])
  })

  it('sends only the changed settings', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
      http.patch(`${API}/servers/${SERVER_ID}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(serverDto({ settings: { ...serverDto().settings, motd: 'Bienvenue' } }))
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}/settings/general/server`)

    const motd = await screen.findByLabelText("Message d'accueil")
    await user.clear(motd)
    await user.type(motd, 'Bienvenue')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ settings: { motd: 'Bienvenue' } }))
    expect(await screen.findByText('Réglages enregistrés, serveur redéployé.')).toBeTruthy()
  })

  it('shows an API error under its setting', async () => {
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
      http.patch(`${API}/servers/${SERVER_ID}/settings`, () =>
        problem(422, 'validation_failed', {
          fields: [{ field: 'settings.motd', code: 'invalid_setting', params: { reason: 'max_length' } }],
        }),
      ),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}/settings/general/server`)

    const motd = await screen.findByLabelText("Message d'accueil")
    await user.type(motd, '!')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    expect(await screen.findByText('Valeur invalide.')).toBeTruthy()
  })

  it('rejects a control character before calling the API', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())))
    const { user } = await renderApp(`/servers/${SERVER_ID}/settings/general/server`)

    // Un champ d'une ligne retire lui-même les retours à la ligne ; une tabulation collée passe.
    await user.click(await screen.findByLabelText("Message d'accueil"))
    await user.paste('a	b')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    expect(await screen.findByText('Retours à la ligne et caractères de contrôle interdits.')).toBeTruthy()
  })

  it('shows the translated option of a choice before it is opened', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())))
    await renderApp(`/servers/${SERVER_ID}/settings/general/server`)

    const difficulty = await screen.findByRole('combobox', { name: 'Difficulté' })
    expect(difficulty.textContent?.trim()).toBe('Normale')
  })

  it('locks the settings fixed at deployment', async () => {
    server.use(...signedIn(), ...catalog(), http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())))
    await renderApp(`/servers/${SERVER_ID}/settings/general/version`)

    const loader = await screen.findByRole('combobox', { name: 'Loader' })
    expect(loader.hasAttribute('disabled') || loader.getAttribute('data-disabled') !== null).toBe(true)
    expect(screen.getByText('Fixé au déploiement')).toBeTruthy()
  })

  it('asks for confirmation before a forced stop', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
      http.post(`${API}/servers/${SERVER_ID}/power`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 202 })
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Forcer l’arrêt' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(body).toBeUndefined()
    await user.click(within(dialog).getByRole('button', { name: 'Forcer l’arrêt' }))

    await waitFor(() => expect(body).toEqual({ action: 'kill' }))
  })
})

describe('server reinstallation', () => {
  function setup(body: { value?: unknown }) {
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
      http.post(`${API}/servers/${SERVER_ID}/reinstall`, async ({ request }) => {
        body.value = await request.json()
        return new HttpResponse(null, { status: 202 })
      }),
    )
  }

  it('reinstalls over the existing data', async () => {
    const body: { value?: unknown } = {}
    setup(body)
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Réinstaller' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('button', { name: 'Réinstaller' }))

    await waitFor(() => expect(body.value).toEqual({ mode: 'reinstall' }))
  })

  it('erases everything only once the name is typed', async () => {
    const body: { value?: unknown } = {}
    setup(body)
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Réinstaller' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('radio', { name: /Tout effacer/ }))
    const erase = within(dialog).getByRole('button', { name: 'Tout effacer et réinstaller' })
    expect(erase.hasAttribute('disabled')).toBe(true)

    await user.type(within(dialog).getByLabelText('Tapez « Survie » pour confirmer'), 'Survie')
    await user.click(erase)

    await waitFor(() => expect(body.value).toEqual({ mode: 'reset' }))
  })

  it('shows the installation state', async () => {
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([serverDto({ state: 'installing' })])),
    )
    await renderApp('/servers')

    expect(await screen.findByText('Installation')).toBeTruthy()
  })
})
