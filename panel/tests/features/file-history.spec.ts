// Historique des fichiers (ADR-0049) : versions dans l'éditeur, comparaison, restauration.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const FILES = `${API}/servers/${SERVER_ID}/files`
const PATH = '/server/data/server.properties'
const VERSION = '01a17c00-0000-7000-8000-000000000001'

const versions: Schemas['FileHistoryDto'] = {
  path: PATH,
  versions: [
    {
      id: VERSION,
      at_ms: Date.UTC(2026, 9, 31, 20, 15),
      size: 18,
      sha256: 'ab',
      reason: 'write',
      channel: 'sftp',
      author_kind: 'admin',
      author_id: profile.user.id,
    },
    {
      id: '01a17c00-0000-7000-8000-000000000002',
      at_ms: Date.UTC(2026, 9, 30, 8, 0),
      size: 12,
      sha256: 'cd',
      reason: 'start',
      channel: 'kuro',
      author_kind: 'system',
      author_id: null,
    },
  ],
}

function setup() {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
    http.get(FILES, () => HttpResponse.json({ path: '/', entries: [] })),
    http.get(`${FILES}/content`, () =>
      HttpResponse.json({ path: PATH, content: 'motd=broken\n', size: 12, modified_ms: 1, sha256: 'now' }),
    ),
    http.get(`${FILES}/history`, () => HttpResponse.json(versions)),
    http.get(`${FILES}/history/${VERSION}`, () =>
      HttpResponse.json({ path: PATH, content: 'motd=working fine\n', size: 18, modified_ms: 0, sha256: 'ab' }),
    ),
  )
}

const open = async (user: Awaited<ReturnType<typeof renderApp>>['user']) => {
  await user.click(await screen.findByRole('button', { name: 'Historique' }))
  return within(await screen.findByRole('dialog'))
}

describe('file history', () => {
  it('lists the kept versions with who changed them and how', async () => {
    setup()
    const { user } = await renderApp(`/servers/${SERVER_ID}/files?file=${PATH}`)

    const sheet = await open(user)
    expect(await sheet.findByText('Modifié')).toBeTruthy()
    expect(sheet.getByText('SFTP')).toBeTruthy()
    expect(sheet.getByText('par vous')).toBeTruthy()
    expect(sheet.getByText('Au démarrage')).toBeTruthy()
    expect(sheet.getByText('par Kuro')).toBeTruthy()
  })

  it('compares a version with the current file, then restores it', async () => {
    let restored: unknown
    setup()
    server.use(
      http.post(`${FILES}/history/${VERSION}/restore`, async ({ request }) => {
        restored = await request.json()
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}/files?file=${PATH}`)

    const sheet = await open(user)
    await user.click(await sheet.findByText('Modifié'))
    // Les deux côtés de la comparaison : la version gardée et l'actuelle.
    const diff = await waitFor(() => {
      const element = document.querySelector<HTMLElement>('[data-slot="version-diff"]')
      if (!element?.textContent?.includes('motd=working fine')) throw new Error('diff not ready')
      return element
    })
    expect(diff.textContent).toContain('motd=broken')

    await user.click(sheet.getByRole('button', { name: 'Restaurer cette version' }))
    const confirm = within(await screen.findByRole('alertdialog'))
    expect(confirm.getByText(/La version actuelle est gardée dans l’historique/)).toBeTruthy()
    await user.click(confirm.getByRole('button', { name: 'Restaurer cette version' }))

    await waitFor(() => expect(restored).toEqual({ path: PATH }))
    expect(await screen.findByText(/restaurée\./)).toBeTruthy()
  })

  it('cannot restore without the write permission', async () => {
    setup()
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read', 'servers.files.read'] }),
      ),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}/files?file=${PATH}`)

    const sheet = await open(user)
    await user.click(await sheet.findByText('Modifié'))
    await waitFor(() => expect(document.querySelector('[data-slot="version-diff"]')).toBeTruthy())
    expect(sheet.queryByRole('button', { name: 'Restaurer cette version' })).toBeNull()
  })
})

describe('deleted files', () => {
  it('lists the deleted files of a folder and brings one back', async () => {
    let restored: unknown
    setup()
    server.use(
      http.get(`${FILES}/deleted`, () =>
        HttpResponse.json({
          path: '/server/data',
          files: [{ path: '/server/data/whitelist.json', version: { ...versions.versions[0], reason: 'delete' } }],
        }),
      ),
      http.post(`${FILES}/history/${VERSION}/restore`, async ({ request }) => {
        restored = await request.json()
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}/files?path=/server/data`)

    await user.click(await screen.findByRole('button', { name: /Fichier supprimé récemment \(1\)/ }))
    expect(await screen.findByText('whitelist.json')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Restaurer' }))

    await waitFor(() => expect(restored).toEqual({ path: '/server/data/whitelist.json' }))
    expect(await screen.findByText('whitelist.json restauré.')).toBeTruthy()
  })
})

