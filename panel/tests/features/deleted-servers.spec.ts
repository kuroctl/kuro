// Serveurs supprimés (ADR-0059) : sauvegardes gardées, reprises par un serveur du même nœud.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { NODE_ID, SERVER_ID, serverDto } from '../support/servers'

const DELETED_ID = '01a17a00-0000-7000-8000-0000000000dd'

const deletedServer = {
  id: DELETED_ID,
  name: 'Ancienne survie',
  prefab_id: 'minecraft-java',
  prefab_version: '1.0.0',
  owner_ref: 'cloud:42',
  node_id: NODE_ID,
  node_name: 'dev-1',
  deleted_at: '2026-11-01T10:00:00Z',
  purge_at: '2026-11-08T10:00:00Z',
  backups: [{ id: '01a17a00-0000-7000-8000-0000000000b1', name: 'Nuit', size_bytes: 2_097_152, created_at: '2026-10-31T03:00:00Z' }],
}

describe('deleted servers', () => {
  it('lists kept backups and lets a server of the same node take them over', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/servers`, () => HttpResponse.json([serverDto(), serverDto({ id: 'other', name: 'Ailleurs', node: { id: 'elsewhere', name: 'dev-2' } })])),
      http.get(`${API}/servers/deleted`, () => HttpResponse.json([deletedServer])),
      http.post(`${API}/servers/deleted/${DELETED_ID}/adopt`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ adopted: 1 })
      }),
    )
    const { user } = await renderApp('/servers/deleted')

    const row = (await screen.findByText('Ancienne survie')).closest('tr')!
    expect(within(row).getByText('cloud:42')).toBeTruthy()
    expect(within(row).getByText(/1 sauvegarde/)).toBeTruthy()
    await user.click(within(row).getByRole('button', { name: 'Reprendre' }))

    const dialog = await screen.findByRole('dialog')
    const select = within(dialog).getByLabelText('Serveur') as HTMLSelectElement
    // Seul le serveur du même nœud est proposé.
    expect(within(select).queryByText('Ailleurs')).toBeNull()
    await user.selectOptions(select, SERVER_ID)
    await user.click(within(dialog).getByRole('button', { name: 'Reprendre' }))

    await waitFor(() => expect(body).toEqual({ server_id: SERVER_ID }))
    expect(await screen.findByText('1 sauvegarde reprise.')).toBeTruthy()
  })
})
