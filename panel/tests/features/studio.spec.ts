// Prefab Studio : brouillons, édition validée pendant la saisie, problèmes situés dans le
// manifest, aperçu du formulaire, images, enregistrement, publication, permission.
import { currentCompletions, startCompletion } from '@codemirror/autocomplete'
import { EditorView } from '@codemirror/view'
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

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
    await user.click(within(dialog).getByRole('combobox'))
    await user.click(await screen.findByRole('option', { name: 'Copie de minecraft-java@1.0.0' }))
    await user.click(within(dialog).getByRole('button', { name: 'Créer' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/prefabs/studio/${DRAFT_ID}`))
    expect(body).toEqual({ from: { id: 'minecraft-java', version: '1.0.0' }, compose: null, egg: null })
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

  it('completes the manifest keys and values from the published schema', async () => {
    setup()
    server.use(
      http.get(`${API}/studio/schema`, () =>
        HttpResponse.json({
          type: 'object',
          properties: {
            category: { enum: ['game', 'web'] },
            install: { $ref: '#/definitions/install' },
          },
          definitions: {
            install: { type: 'object', properties: { steps: { type: 'array', items: { $ref: '#/definitions/step' } } } },
            step: { oneOf: [{ properties: { download: { type: 'object' } } }, { properties: { extract: { type: 'object' } } }] },
          },
        }),
      ),
    )
    await renderApp(`/prefabs/studio/${DRAFT_ID}`)
    await screen.findByText('Prefab valide, prêt à publier.')
    // L'éditeur est recréé une fois le schéma chargé.
    await waitFor(() => expect(document.querySelector('.cm-content')).toBeTruthy())

    const complete = async (text: string) => {
      await typeInEditor(text)
      const view = EditorView.findFromDOM(document.querySelector('.cm-content') as HTMLElement)!
      view.dispatch({ selection: { anchor: text.length } })
      startCompletion(view)
      return waitFor(() => {
        const labels = currentCompletions(view.state).map((option) => option.label)
        if (!labels.length) throw new Error('no completion yet')
        return labels
      })
    }

    expect(await complete('install:\n  steps:\n    - ')).toEqual(['download', 'extract'])
    expect(await complete('category: ')).toEqual(['game', 'web'])
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
