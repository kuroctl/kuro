// Mise à jour d'un serveur vers une version plus récente de son prefab (ADR-0024) : bandeau,
// aperçu, sauvegarde préalable, blocage, mise à jour groupée depuis la fiche d'une version.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, localeFr, manifest, prefabSummary, SERVER_ID, serverDto } from '../support/servers'

const preview: Schemas['UpgradePreviewDto'] = {
  from: '1.0.0',
  to: '1.2.0',
  renamed: [{ from: 'motd', to: 'welcome' }],
  removed: [],
  added: ['view_distance'],
  ports_added: [],
  ports_removed: [],
  images: [{ service: 'server', from: 'itzg:1', to: 'itzg:2' }],
  reinstall: false,
  blockers: [],
}

function withUpgrade(previewed = preview) {
  server.use(
    ...signedIn(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
    http.get(`${API}/servers/${SERVER_ID}/upgrades`, () => HttpResponse.json({ current: '1.0.0', available: ['1.1.0', '1.2.0'] })),
    http.get(`${API}/servers/${SERVER_ID}/upgrade`, ({ request }) =>
      HttpResponse.json({ ...previewed, to: new URL(request.url).searchParams.get('to') }),
    ),
    ...catalog(),
  )
}

describe('server upgrade', () => {
  it('offers the latest version, previews it and backs up first by default', async () => {
    let body: unknown
    withUpgrade()
    server.use(
      http.post(`${API}/servers/${SERVER_ID}/upgrade`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(preview, { status: 202 })
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByText('Version 1.2.0 disponible (actuelle : 1.0.0).')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Mettre à jour' }))
    const dialog = await screen.findByRole('dialog')
    expect(await within(dialog).findByText('Réglage motd renommé en welcome')).toBeTruthy()
    expect(within(dialog).getByText('Image du service server : itzg:2')).toBeTruthy()
    expect(within(dialog).getByRole('switch', { name: /Sauvegarder avant/ }).getAttribute('aria-checked')).toBe('true')

    await user.click(within(dialog).getByRole('button', { name: 'Mettre à jour' }))
    await waitFor(() => expect(body).toEqual({ to: '1.2.0', backup: true }))
    expect(await screen.findByText('Sauvegarde puis mise à jour vers 1.2.0 lancées.')).toBeTruthy()
  })

  it('cannot confirm a blocked upgrade and says why', async () => {
    withUpgrade({ ...preview, blockers: [{ kind: 'setting', field: 'difficulty', reason: 'options' }] })
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Mettre à jour' }))
    const dialog = await screen.findByRole('dialog')
    expect(
      await within(dialog).findByText('Le réglage difficulty n’est plus valide dans cette version (option retirée) : modifiez-le d’abord.'),
    ).toBeTruthy()
    expect((within(dialog).getByRole('button', { name: 'Mettre à jour' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('shows no banner without a newer version', async () => {
    server.use(...signedIn(), http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())), ...catalog())
    await renderApp(`/servers/${SERVER_ID}`)
    await screen.findByText('Minecraft Java Edition', { selector: 'p' })
    expect(screen.queryByText(/disponible \(actuelle/)).toBeNull()
  })
})

describe('bulk upgrade', () => {
  it('upgrades the older servers of a version, blocked ones left out', async () => {
    let body: unknown
    const v2 = { ...prefabSummary, version: '1.2.0' }
    server.use(
      ...signedIn(),
      http.get(`${API}/prefabs`, () => HttpResponse.json([prefabSummary, v2])),
      http.get(`${API}/prefabs/minecraft-java/:version`, ({ params }) =>
        HttpResponse.json({ ...v2, version: params.version, manifest: { ...manifest, version: params.version }, locales: ['fr'] }),
      ),
      http.get(`${API}/prefabs/minecraft-java/:version/locales/fr`, () => HttpResponse.json(localeFr)),
      http.get(`${API}/prefabs/minecraft-java/:version/servers`, () => HttpResponse.json([])),
      http.get(`${API}/prefabs/minecraft-java/1.2.0/upgrades`, () =>
        HttpResponse.json([
          { server_id: SERVER_ID, name: 'Survie', version: '1.0.0', state: 'running', preview },
          {
            server_id: 'blocked-id',
            name: 'Créatif',
            version: '1.0.0',
            state: 'offline',
            preview: { ...preview, blockers: [{ kind: 'not_deployable' }] },
          },
        ]),
      ),
      http.post(`${API}/servers/upgrade`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json([{ server_id: SERVER_ID, started: true, error: null }], { status: 202 })
      }),
      ...catalog(),
    )
    const { user } = await renderApp('/prefabs/minecraft-java?version=1.2.0')

    await user.click(await screen.findByRole('tab', { name: 'Serveurs' }))
    expect(await screen.findByText('Mettre à jour vers 1.2.0')).toBeTruthy()
    expect((screen.getByRole('switch', { name: 'Créatif' }) as HTMLButtonElement).disabled).toBe(true)
    await user.click(screen.getByRole('button', { name: 'Mettre à jour 1 serveur' }))

    await waitFor(() => expect(body).toEqual({ servers: [SERVER_ID], to: '1.2.0', backup: true }))
    expect(await screen.findByText('Mise à jour de 1 serveur vers 1.2.0 lancée.')).toBeTruthy()
  })
})

describe('server upgrade, once done', () => {
  it('drops the banner when the server reaches the new version, without reloading', async () => {
    let version = '1.0.0'
    server.use(
      ...signedIn(),
      http.get(`${API}/servers/${SERVER_ID}`, () =>
        HttpResponse.json(serverDto({ prefab: { id: 'minecraft-java', version } })),
      ),
      http.get(`${API}/servers/${SERVER_ID}/upgrades`, () =>
        HttpResponse.json({ current: version, available: version === '1.0.0' ? ['1.2.0'] : [] }),
      ),
      ...catalog(),
    )
    await renderApp(`/servers/${SERVER_ID}`)
    expect(await screen.findByText('Version 1.2.0 disponible (actuelle : 1.0.0).')).toBeTruthy()

    // La mise à jour s'achève en arrière-plan : seul le serveur est relu.
    version = '1.2.0'
    const { queryClient } = await import('@/shared/query/client')
    await queryClient.invalidateQueries({ queryKey: ['servers'] })
    await waitFor(() => expect(screen.queryByText(/disponible \(actuelle/)).toBeNull())
  })
})
