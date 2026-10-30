// Fichiers d'un serveur : navigation dans les volumes, fichiers du prefab, éditeur, conflits.
import { EditorView } from '@codemirror/view'
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, manifest, prefabSummary, SERVER_ID, serverDto } from '../support/servers'

const FILES = `${API}/servers/${SERVER_ID}/files`
const PAGE = `/servers/${SERVER_ID}/files`
const PROPERTIES = '/server/data/server.properties'

const listings: Record<string, unknown[]> = {
  '/': [{ name: 'server', kind: 'directory', size: 0, modified_ms: 0 }],
  '/server/data': [
    { name: 'world', kind: 'directory', size: 4096, modified_ms: 1_793_000_000_000 },
    { name: 'server.properties', kind: 'file', size: 10, modified_ms: 1_793_000_000_000 },
    { name: 'latest.log', kind: 'symlink', size: 12, modified_ms: 1_793_000_000_000 },
    { name: 'world.zip', kind: 'file', size: 50 * 1024 * 1024, modified_ms: 1_793_000_000_000 },
  ],
}

function setup() {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
    http.get(FILES, ({ request }) => {
      const path = new URL(request.url).searchParams.get('path') ?? '/'
      return HttpResponse.json({ path, entries: listings[path] ?? [] })
    }),
    http.get(`${FILES}/content`, ({ request }) =>
      HttpResponse.json({
        path: new URL(request.url).searchParams.get('path'),
        content: 'motd=Kuro\n',
        size: 10,
        modified_ms: 1,
        sha256: 'sha-1',
      }),
    ),
  )
}

/** Liste du dossier courant (l'arbre, à côté, montre les mêmes noms). */
const browser = async () => within(await waitFor(() => document.querySelector<HTMLElement>('[data-slot="file-browser"]')!))
const tree = () => within(screen.getByRole('tree'))

/** Saisie dans l'éditeur, comme le ferait le clavier. */
async function typeInEditor(text: string) {
  const content = await waitFor(() => {
    const element = document.querySelector('.cm-content')
    if (!element) throw new Error('editor not ready')
    return element as HTMLElement
  })
  const view = EditorView.findFromDOM(content)!
  view.dispatch({ changes: { from: view.state.doc.length, insert: text } })
}

describe('server files', () => {
  it('lists the prefab files first, then browses the volumes', async () => {
    setup()
    const { user, router } = await renderApp(PAGE)

    expect(await screen.findByText(PROPERTIES)).toBeTruthy()
    expect(screen.getByText('Configuration du serveur.')).toBeTruthy()

    await user.click(await (await browser()).findByRole('button', { name: 'server' }))
    await waitFor(() => expect(router.currentRoute.value.query.path).toBe('/server'))
    await router.push({ query: { path: '/server/data' } })

    expect(await (await browser()).findByRole('button', { name: 'server.properties' })).toBeTruthy()
    expect(screen.getByText('Lien')).toBeTruthy()
    expect(screen.getByText('Trop gros pour l’éditeur')).toBeTruthy()
  })

  it('lists the registration files Kuro keeps, under the world of the server', async () => {
    setup()
    const register = { file: '/data/worlds/{{ settings.level_name }}/world_behavior_packs.json' }
    server.use(
      http.get(`${API}/prefabs/minecraft-java/1.0.0`, () =>
        HttpResponse.json({
          ...prefabSummary,
          manifest: { ...manifest, content: { addons: { kind: 'addons', service: 'server', register: [register, register] } } },
          locales: ['en', 'fr'],
        }),
      ),
      http.get(`${API}/servers/${SERVER_ID}`, () =>
        HttpResponse.json(serverDto({ state: 'running', settings: { level_name: 'Bedrock level' } })),
      ),
    )
    await renderApp(PAGE)

    // Deux déclarations, un seul fichier.
    const paths = await screen.findAllByText('/server/data/worlds/Bedrock level/world_behavior_packs.json')
    expect(paths).toHaveLength(1)
    expect(screen.getByText('Tenu par Kuro')).toBeTruthy()
    expect(screen.getByText(/réécrits à chaque démarrage/)).toBeTruthy()
  })

  it('saves an edited file with the fingerprint it was read with', async () => {
    let body: unknown
    setup()
    server.use(
      http.put(`${FILES}/content`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`${PAGE}?file=${PROPERTIES}`)

    await typeInEditor('max-players=5\n')
    expect(await screen.findByText('Non enregistré')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({ path: PROPERTIES, content: 'motd=Kuro\nmax-players=5\n', if_sha256: 'sha-1' }),
    )
    expect(await screen.findByText('Fichier enregistré.')).toBeTruthy()
  })

  it('offers to overwrite a file changed since it was opened', async () => {
    const bodies: unknown[] = []
    setup()
    server.use(
      http.put(`${FILES}/content`, async ({ request }) => {
        bodies.push(await request.json())
        return bodies.length === 1 ? problem(409, 'file_conflict') : new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`${PAGE}?file=${PROPERTIES}`)

    await typeInEditor('pvp=false\n')
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText('Le fichier a changé')).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Écraser' }))

    await waitFor(() => expect(bodies).toHaveLength(2))
    expect(bodies[1]).toMatchObject({ if_sha256: null })
  })

  it('asks before leaving unsaved changes', async () => {
    setup()
    const { user, router } = await renderApp(`${PAGE}?file=${PROPERTIES}`)

    await typeInEditor('x')
    await user.click(await screen.findByRole('button', { name: 'Fermer le fichier' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: 'Rester' }))

    expect(router.currentRoute.value.query.file).toBe(PROPERTIES)
  })

  it('shows why a file cannot be opened', async () => {
    setup()
    server.use(http.get(`${FILES}/content`, () => problem(422, 'file_not_text')))
    await renderApp(`${PAGE}?file=/server/data/level.dat`)

    expect((await screen.findByRole('alert')).textContent).toContain('n’est pas un fichier texte')
  })

  it('creates a folder after checking its name', async () => {
    let body: unknown
    setup()
    server.use(
      http.post(`${FILES}/directories`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 201 })
      }),
    )
    const { user } = await renderApp(`${PAGE}?path=/server/data`)

    await user.click(await screen.findByRole('button', { name: 'Nouveau dossier' }))
    const input = await screen.findByLabelText('Nom')
    await user.type(input, '../etc')
    await user.click(screen.getByRole('button', { name: 'Créer' }))
    expect(await screen.findByText(/Nom invalide/)).toBeTruthy()
    expect(body).toBeUndefined()

    await user.clear(input)
    await user.type(input, 'plugins')
    await user.click(screen.getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(body).toEqual({ path: '/server/data/plugins' }))
  })

  it('deletes after an explicit confirmation', async () => {
    let deleted: string | null = null
    setup()
    server.use(
      http.delete(FILES, ({ request }) => {
        deleted = new URL(request.url).searchParams.get('path')
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`${PAGE}?path=/server/data`)

    await user.click(await screen.findByRole('button', { name: 'Actions sur world' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Supprimer' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText('Le dossier et tout son contenu sont supprimés définitivement.')).toBeTruthy()
    expect(deleted).toBeNull()
    await user.click(within(dialog).getByRole('button', { name: 'Supprimer définitivement' }))

    await waitFor(() => expect(deleted).toBe('/server/data/world'))
  })

  it('shows the volumes as a tree that opens folders and files', async () => {
    setup()
    listings['/server'] = [{ name: 'data', kind: 'directory', size: 0, modified_ms: 0 }]
    const { user, router } = await renderApp(`${PAGE}?path=/server/data`)

    // Les dossiers menant à l'emplacement courant sont ouverts.
    const data = await tree().findByRole('treeitem', { name: 'data', expanded: true })
    expect(await within(data).findByRole('treeitem', { name: 'world', expanded: false })).toBeTruthy()
    expect(within(data).queryByRole('treeitem', { name: 'latest.log' })).toBeNull()

    await user.click(within(data).getByRole('treeitem', { name: 'server.properties' }))
    await waitFor(() => expect(router.currentRoute.value.query.file).toBe(PROPERTIES))
    expect(tree().getByRole('treeitem', { name: 'server.properties', selected: true })).toBeTruthy()

    await user.click(tree().getByRole('button', { name: 'world' }))
    await waitFor(() => expect(router.currentRoute.value.query.path).toBe('/server/data/world'))
    expect(await tree().findByText('Dossier vide')).toBeTruthy()
    delete listings['/server']
  })

  it('changes nothing at the level of services and volumes', async () => {
    setup()
    await renderApp(PAGE)

    await (await browser()).findByRole('button', { name: 'server' })
    expect(screen.queryByRole('button', { name: 'Nouveau dossier' })).toBeNull()
    expect(screen.queryByRole('button', { name: /Actions sur/ })).toBeNull()
  })

  it('uploads a file into the current folder', async () => {
    setup()
    let path: string | null = null
    let size = 0
    server.use(
      http.put(`${FILES}/upload`, async ({ request }) => {
        path = new URL(request.url).searchParams.get('path')
        size = (await request.arrayBuffer()).byteLength
        return HttpResponse.json({ path, size, sha256: 'a'.repeat(64) }, { status: 201 })
      }),
    )
    const { user } = await renderApp(`${PAGE}?path=/server/data`)

    const input = await screen.findByLabelText('Envoyer un fichier', { selector: 'input' })
    await user.upload(input as HTMLInputElement, new File(['world bytes'], 'world.zip'))

    await waitFor(() => expect(path).toBe('/server/data/world.zip'))
    expect(size).toBe(11)
    expect(await screen.findByText('world.zip envoyé.')).toBeTruthy()
  })
})
