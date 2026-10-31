// Prefab Studio : brouillons, édition validée pendant la saisie, problèmes situés dans le
// manifest, aperçu du formulaire, images, enregistrement, publication, permission.
import { CompletionContext } from '@codemirror/autocomplete'
import { EditorState } from '@codemirror/state'
import { EditorView } from '@codemirror/view'
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { schemaCompletion } from '@/features/studio/manifestEditor'
import type { Schemas } from '@/shared/api/types'

import { API, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, NODE_ID, prefabSummary, SERVER_ID, serverDto } from '../support/servers'

const DRAFT_ID = '01a19000-0000-7000-8000-000000000001'
const MANIFEST = `schema: kuro.prefab/v1
id: my-prefab
version: 0.1.0
settings:
  general:
    server:
      fields:
        max_players: { type: int, default: 10, min: 1, max: 100 }
`

const draft: Schemas['DraftDto'] = {
  id: DRAFT_ID,
  prefab_id: 'my-prefab',
  prefab_version: '0.1.0',
  base_id: null,
  base_version: null,
  assets: [],
  created_at: '2026-10-28T10:00:00Z',
  updated_at: '2026-10-28T10:00:00Z',
  files: {
    'prefab.yaml': MANIFEST,
    'locales/fr.json': JSON.stringify({ prefab: { name: 'Mon prefab' }, fields: { max_players: { label: 'Joueurs maximum' } } }),
  },
  check: { valid: true, reason: null, issues: [] },
}

function setup(drafts: Schemas['DraftSummaryDto'][] = [draft]) {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/prefabs`, () => HttpResponse.json([prefabSummary])),
    http.get(`${API}/prefabs/search`, () =>
      HttpResponse.json({
        items: [{ id: 'minecraft-java', names: { fr: 'Minecraft: Java Edition' }, versions: ['1.1.0', '1.0.0'] }],
        total: 1,
        page: 1,
        per_page: 8,
      }),
    ),
    http.get(`${API}/studio/schema`, () =>
      HttpResponse.json({ type: 'object', properties: { id: { type: 'string', description: 'Identifiant' } } }),
    ),
    http.get(`${API}/studio/drafts`, () => HttpResponse.json(drafts)),
    http.get(`${API}/studio/drafts/${DRAFT_ID}`, () => HttpResponse.json(draft)),
  )
}

/** Remplace le texte de l'éditeur ouvert, comme une saisie. */
async function typeInEditor(text: string) {
  const content = await waitFor(() => {
    const element = document.querySelector('.cm-content')
    if (!element) throw new Error('editor not ready')
    return element as HTMLElement
  })
  const view = EditorView.findFromDOM(content)!
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } })
}

describe('prefab studio', () => {
  it('lists the drafts and creates one from a catalog version', async () => {
    setup()
    let body: unknown
    server.use(
      http.post(`${API}/studio/drafts`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(draft, { status: 201 })
      }),
    )
    const { user, router } = await renderApp('/prefabs/studio')

    expect(await screen.findByRole('link', { name: /my-prefab/ })).toBeTruthy()
    await user.click(screen.getAllByRole('button', { name: 'Nouveau brouillon' })[0]!)
    const dialog = await screen.findByRole('dialog')
    await user.click(await within(dialog).findByRole('button', { name: /Minecraft: Java Edition/ }))
    // La dernière version est proposée ; une plus ancienne peut être copiée.
    expect(within(dialog).getByRole('radio', { name: /1\.1\.0/ }).getAttribute('aria-checked')).toBe('true')
    await user.click(within(dialog).getByRole('radio', { name: /1\.0\.0/ }))
    await user.click(within(dialog).getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/prefabs/studio/${DRAFT_ID}`))
    expect(body).toEqual({ from: { id: 'minecraft-java', version: '1.0.0' }, compose: null, egg: null })
  })

  it('searches the catalog page by page on the server, or starts from the example', async () => {
    setup()
    const searches: string[] = []
    let body: unknown
    server.use(
      http.get(`${API}/prefabs/search`, ({ request }) => {
        const url = new URL(request.url)
        searches.push(`${url.searchParams.get('q')}:${url.searchParams.get('page')}`)
        const page = Number(url.searchParams.get('page'))
        return HttpResponse.json({
          items: [{ id: `game-${page}`, names: { fr: `Jeu ${page}` }, versions: ['1.0.0'] }],
          total: 9,
          page,
          per_page: 8,
        })
      }),
      http.post(`${API}/studio/drafts`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(draft, { status: 201 })
      }),
    )
    const { user } = await renderApp('/prefabs/studio')
    await user.click((await screen.findAllByRole('button', { name: 'Nouveau brouillon' }))[0]!)
    const dialog = await screen.findByRole('dialog')

    expect(await within(dialog).findByText('Page 1 sur 2')).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Page suivante' }))
    expect(await within(dialog).findByRole('button', { name: /Jeu 2/ })).toBeTruthy()
    await user.type(within(dialog).getByRole('searchbox', { name: 'Rechercher un prefab' }), 'ark')
    await waitFor(() => expect(searches).toContain('ark:1'))
    expect(searches).toContain(':2')

    await user.click(within(dialog).getByRole('button', { name: 'Partir du prefab d’exemple' }))
    await waitFor(() => expect(body).toEqual({ from: null, compose: null, egg: null }))
  })

  it('checks the manifest while typing and points at each problem', async () => {
    setup()
    let validated: Record<string, string> | undefined
    server.use(
      http.post(`${API}/studio/validate`, async ({ request }) => {
        validated = ((await request.json()) as { files: Record<string, string> }).files
        return HttpResponse.json({ valid: false, reason: null, issues: [{ path: 'services.server.image', message: 'missing tag' }] })
      }),
    )
    await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    expect(await screen.findByText('Prefab valide, prêt à publier.')).toBeTruthy()

    await typeInEditor(`${MANIFEST}services:\n  server:\n    image: debian\n`)

    expect(await screen.findByText('Un problème', {}, { timeout: 3000 })).toBeTruthy()
    expect(screen.getByRole('button', { name: /services\.server\.image/ })).toBeTruthy()
    expect(validated?.['prefab.yaml']).toContain('image: debian')
    expect(screen.getByRole('button', { name: 'Publier' }).hasAttribute('disabled')).toBe(true)
    expect(screen.getByText('Non enregistré')).toBeTruthy()
  })

  it('saves the files, then publishes the draft into the catalog', async () => {
    setup()
    let saved: Record<string, string> | undefined
    let published = false
    server.use(
      http.post(`${API}/studio/validate`, () => HttpResponse.json({ valid: true, reason: null, issues: [] })),
      http.put(`${API}/studio/drafts/${DRAFT_ID}`, async ({ request }) => {
        saved = ((await request.json()) as { files: Record<string, string> }).files
        return HttpResponse.json({ ...draft, files: saved })
      }),
      http.post(`${API}/studio/drafts/${DRAFT_ID}/publish`, () => {
        published = true
        return HttpResponse.json({ ...prefabSummary, id: 'my-prefab', version: '0.1.0', source: 'uploaded' }, { status: 201 })
      }),
    )
    const { user, router } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await screen.findByText('Prefab valide, prêt à publier.')

    await typeInEditor(MANIFEST.replace('default: 10', 'default: 12'))
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))
    await waitFor(() => expect(saved?.['prefab.yaml']).toContain('default: 12'))

    await waitFor(() => expect(screen.getByRole('button', { name: 'Publier' }).hasAttribute('disabled')).toBe(false))
    await user.click(screen.getByRole('button', { name: 'Publier' }))
    await user.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Publier' }))

    await waitFor(() => expect(router.currentRoute.value.fullPath).toBe('/prefabs/my-prefab?version=0.1.0'))
    expect(published).toBe(true)
  })

  it('exports the saved draft as a .kprefab archive', async () => {
    setup()
    let exported = false
    server.use(
      http.post(`${API}/studio/validate`, () => HttpResponse.json({ valid: true, reason: null, issues: [] })),
      http.get(`${API}/studio/drafts/${DRAFT_ID}/export`, () => {
        exported = true
        return new HttpResponse(new Blob(['PK']), { headers: { 'Content-Type': 'application/zip' } })
      }),
    )
    let downloaded: string | undefined
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      downloaded = this.download
    })
    const { user } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await screen.findByText('Prefab valide, prêt à publier.')

    await user.click(screen.getByRole('button', { name: 'Exporter' }))

    await waitFor(() => expect(downloaded).toBe('my-prefab-0.1.0.kprefab'))
    expect(exported).toBe(true)
    click.mockRestore()
  })

  it('completes the manifest keys and values from the published schema', () => {
    // La source de complétion est exercée sur un état d'éditeur, sans vue : la bulle de
    // complétion de CodeMirror a besoin d'une mise en page que jsdom n'a pas.
    const schema = {
      type: 'object',
      properties: {
        category: { enum: ['game', 'web'] },
        install: { $ref: '#/definitions/install' },
      },
      definitions: {
        install: { type: 'object', properties: { steps: { type: 'array', items: { $ref: '#/definitions/step' } } } },
        step: { oneOf: [{ properties: { download: { type: 'object' } } }, { properties: { extract: { type: 'object' } } }] },
      },
    }
    const complete = (text: string) => {
      const state = EditorState.create({ doc: text })
      const result = schemaCompletion(schema as never)(new CompletionContext(state, text.length, true))
      return result?.options.map((option) => option.label) ?? []
    }

    expect(complete('install:\n  steps:\n    - ')).toEqual(['download', 'extract'])
    expect(complete('category: ')).toEqual(['game', 'web'])
  })

  it('previews the settings form with the draft translations', async () => {
    setup()
    const { user } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)

    await user.click(await screen.findByRole('tab', { name: 'Aperçu' }))

    expect(await screen.findByText('Mon prefab')).toBeTruthy()
    expect(screen.getByLabelText('Joueurs maximum')).toBeTruthy()
  })

  it('uploads an image for the draft', async () => {
    setup()
    let uploaded: string | undefined
    server.use(
      http.put(`${API}/studio/drafts/${DRAFT_ID}/assets/:kind`, ({ params }) => {
        uploaded = String(params.kind)
        return new HttpResponse(null, { status: 204 })
      }),
      http.get(`${API}/studio/drafts/${DRAFT_ID}/assets/:kind`, () =>
        HttpResponse.arrayBuffer(new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer, { headers: { 'Content-Type': 'image/png' } }),
      ),
    )
    const { user } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await user.click(await screen.findByRole('tab', { name: 'Images' }))

    const input = await screen.findByLabelText('Choisir l’image : icône')
    await user.upload(input as HTMLInputElement, new File([new Uint8Array([0x89, 0x50, 0x4e, 0x47])], 'icon.png', { type: 'image/png' }))

    await waitFor(() => expect(uploaded).toBe('icon'))
    expect(await screen.findByRole('button', { name: 'Retirer l’image : icône' })).toBeTruthy()
  })

  it('offers no Steam import when the manifest declares no Steam game', async () => {
    setup()
    const { user } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await user.click(await screen.findByRole('tab', { name: 'Images' }))

    await screen.findByLabelText('Choisir l’image : icône')
    expect(screen.queryByRole('button', { name: /Importer depuis Steam/ })).toBeNull()
  })

  it('imports the images of the declared Steam game', async () => {
    setup()
    let imported = false
    server.use(
      http.get(`${API}/studio/drafts/${DRAFT_ID}`, () =>
        HttpResponse.json({ ...draft, files: { ...draft.files, 'prefab.yaml': `${MANIFEST}steam:
  app: 730
` } }),
      ),
      http.post(`${API}/studio/drafts/${DRAFT_ID}/assets/steam`, () => {
        imported = true
        return HttpResponse.json({
          app: 730,
          name: 'Counter-Strike 2',
          imported: ['icon', 'thumbnail'],
          sources: { icon: 'steamgriddb', thumbnail: 'steam' },
          missing: ['cover'],
        })
      }),
      http.get(`${API}/studio/drafts/${DRAFT_ID}/assets/:kind`, () =>
        HttpResponse.arrayBuffer(new Uint8Array([0x89, 0x50, 0x4e, 0x47]).buffer, { headers: { 'Content-Type': 'image/png' } }),
      ),
    )
    const { user } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await user.click(await screen.findByRole('tab', { name: 'Images' }))

    await user.click(await screen.findByRole('button', { name: 'Importer depuis Steam (730)' }))

    await waitFor(() => expect(imported).toBe(true))
    expect(await screen.findByText('Images de Counter-Strike 2 importées ; introuvables : bannière.')).toBeTruthy()
    expect(await screen.findByText('Depuis SteamGridDB : icône.')).toBeTruthy()
    expect(await screen.findByRole('button', { name: 'Retirer l’image : icône' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Retirer l’image : bannière' })).toBeNull()
  })

  it('converts a docker-compose, lists what was refused, then creates the draft', async () => {
    setup([])
    let converted: unknown
    let created: unknown
    server.use(
      http.post(`${API}/studio/compose`, async ({ request }) => {
        converted = await request.json()
        return HttpResponse.json({
          id: 'my-nginx',
          files: { 'prefab.yaml': MANIFEST },
          notes: [
            { path: 'services.web.privileged', kind: 'refused', message: 'un conteneur privilégié ouvre l’hôte' },
            { path: 'services.web.restart', kind: 'ignored', message: 'géré par Kuro' },
          ],
          check: { valid: true, reason: null, issues: [] },
        })
      }),
      http.post(`${API}/studio/drafts`, async ({ request }) => {
        created = await request.json()
        return HttpResponse.json(draft, { status: 201 })
      }),
    )
    const { user, router } = await renderApp('/prefabs/studio')

    await user.click(await screen.findByRole('button', { name: 'Importer un docker-compose' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('docker-compose.yml'), 'services: web')
    await user.type(within(dialog).getByLabelText('Identifiant du prefab (facultatif)'), 'my-nginx')
    await user.click(within(dialog).getByRole('button', { name: 'Convertir' }))

    expect(await within(dialog).findByText('my-nginx : prefab valide.')).toBeTruthy()
    expect(within(dialog).getByText('Refusé')).toBeTruthy()
    expect(within(dialog).getByText('services.web.privileged')).toBeTruthy()
    expect(converted).toEqual({ text: 'services: web', id: 'my-nginx' })

    await user.click(within(dialog).getByRole('button', { name: 'Créer le brouillon' }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/prefabs/studio/${DRAFT_ID}`))
    expect(created).toEqual({ from: null, compose: { text: 'services: web', id: 'my-nginx' }, egg: null })
  })

  it('converts a Pterodactyl egg, lists what was refused, then creates the draft', async () => {
    setup([])
    let converted: unknown
    let created: unknown
    server.use(
      http.post(`${API}/studio/egg`, async ({ request }) => {
        converted = await request.json()
        return HttpResponse.json({
          id: 'paper',
          files: { 'prefab.yaml': MANIFEST },
          notes: [
            { path: 'scripts.installation.script', kind: 'refused', message: 'installe des paquets système' },
            { path: 'docker_images', kind: 'adapted', message: 'tag à épingler' },
          ],
          check: { valid: true, reason: null, issues: [] },
        })
      }),
      http.post(`${API}/studio/drafts`, async ({ request }) => {
        created = await request.json()
        return HttpResponse.json(draft, { status: 201 })
      }),
    )
    const { user, router } = await renderApp('/prefabs/studio')

    await user.click(await screen.findByRole('button', { name: 'Convertir un egg Pterodactyl' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByLabelText('Egg (JSON)'))
    await user.paste('{"meta":{"version":"PTDL_v2"}}')
    await user.click(within(dialog).getByRole('button', { name: 'Convertir' }))

    expect(await within(dialog).findByText('paper : prefab valide.')).toBeTruthy()
    expect(within(dialog).getByText('scripts.installation.script')).toBeTruthy()
    expect(within(dialog).getByText('Adapté')).toBeTruthy()
    expect(converted).toEqual({ text: '{"meta":{"version":"PTDL_v2"}}', id: null })

    await user.click(within(dialog).getByRole('button', { name: 'Créer le brouillon' }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/prefabs/studio/${DRAFT_ID}`))
    expect(created).toEqual({ from: null, compose: null, egg: { text: '{"meta":{"version":"PTDL_v2"}}' } })
  })

  it('tries the saved draft on a test node and opens the trial server', async () => {
    setup()
    let body: unknown
    server.use(
      http.get(`${API}/nodes`, () => HttpResponse.json([{ ...node, trials: true }, { ...node, id: 'other', name: 'prod-1', display_name: null }])),
      http.post(`${API}/studio/drafts/${DRAFT_ID}/trial`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ server_id: SERVER_ID, version: '0.1.0-trial.1', expires_at: '2026-10-28T12:00:00Z' }, { status: 201 })
      }),
    )
    const { user, router } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await screen.findByText('Prefab valide, prêt à publier.')

    await user.click(screen.getByRole('button', { name: 'Essayer' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByRole('combobox').textContent).toContain('Dev 1')
    await user.click(within(dialog).getByRole('button', { name: 'Lancer l’essai' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/servers/${SERVER_ID}`))
    expect(body).toEqual({ node_id: NODE_ID, name: 'Essai de my-prefab' })
  })

  it('says where to mark a test node when there is none', async () => {
    setup()
    const { user } = await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await screen.findByText('Prefab valide, prêt à publier.')

    await user.click(screen.getByRole('button', { name: 'Essayer' }))

    expect(await screen.findByText(/Aucun nœud de test en ligne/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Lancer l’essai' })).toBeNull()
  })

  it('shows the running trial of the draft', async () => {
    setup()
    server.use(
      http.get(`${API}/studio/drafts/${DRAFT_ID}`, () =>
        HttpResponse.json({ ...draft, trial: { server_id: SERVER_ID, name: 'Essai de my-prefab', state: 'running', expires_at: '2026-10-28T12:00:00Z' } }),
      ),
    )
    await renderApp(`/prefabs/studio/${DRAFT_ID}`)

    expect(await screen.findByRole('link', { name: 'Essai de my-prefab' })).toBeTruthy()
  })

  it('marks a node as a test node', async () => {
    setup()
    let patch: unknown
    server.use(
      http.get(`${API}/nodes/${NODE_ID}`, () => HttpResponse.json(node)),
      http.get(`${API}/nodes/${NODE_ID}/allocations`, () => HttpResponse.json([])),
      http.get(`${API}/regions`, () => HttpResponse.json([])),
      http.patch(`${API}/nodes/${NODE_ID}`, async ({ request }) => {
        patch = await request.json()
        return HttpResponse.json({ ...node, trials: true })
      }),
    )
    const { user } = await renderApp(`/servers/nodes/${NODE_ID}`)

    await user.click(await screen.findByRole('switch', { name: 'Nœud de test' }))

    await waitFor(() => expect(patch).toEqual({ trials: true }))
  })

  it('badges a trial server with its end', async () => {
    setup()
    server.use(
      http.get(`${API}/servers/${SERVER_ID}`, () =>
        HttpResponse.json(serverDto({ trial: { draft_id: DRAFT_ID, expires_at: '2026-10-28T12:00:00Z' } })),
      ),
    )
    await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByText(/Essai · jusqu’à/)).toBeTruthy()
  })

  it('is out of reach without the manage permission', async () => {
    setup()
    server.use(http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['prefabs.read'] })))
    await renderApp('/prefabs/studio')

    expect(await screen.findByText('Le Studio demande la permission de gérer les prefabs.')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Nouveau brouillon' })).toBeNull()
  })
})
