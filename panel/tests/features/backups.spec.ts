// Sauvegardes d'un serveur : liste, création, restauration, suppression, téléchargement par ticket.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { API, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const BACKUPS = `/servers/${SERVER_ID}/backups`
const READY_ID = '01a17da3-7b31-744d-850c-000000000001'
const FAILED_ID = '01a17da3-7b31-744d-850c-000000000002'

function backup(overrides: Record<string, unknown> = {}) {
  return {
    id: READY_ID,
    name: 'Avant la mise à jour',
    state: 'ready',
    size_bytes: 2_097_152,
    sha256: 'a'.repeat(64),
    locked: false,
    failure: null,
    created_at: '2026-10-27T10:00:00Z',
    completed_at: '2026-10-27T10:01:00Z',
    ...overrides,
  }
}

function setup(backups = [backup(), backup({ id: FAILED_ID, name: 'Ratée', state: 'failed', size_bytes: null, sha256: null, failure: 'disk full' })]) {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
    http.get(`${API}${BACKUPS}`, () => HttpResponse.json({ limit: 3, backups })),
  )
}

async function openActions(user: Awaited<ReturnType<typeof renderApp>>['user'], name: string) {
  await user.click(await screen.findByRole('button', { name: `Actions sur ${name}` }))
}

describe('server backups', () => {
  it('keeps the server menu', async () => {
    setup()
    await renderApp(BACKUPS)

    const link = await waitFor(() => {
      const nav = screen.queryAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))
      expect(nav).toBeTruthy()
      return within(nav!).getByRole('link', { name: 'Sauvegardes' })
    })
    expect(link.getAttribute('data-active')).toBe('true')
  })

  it('lists the backups with their state, size and the limit', async () => {
    setup()
    await renderApp(BACKUPS)

    expect(await screen.findByText('Avant la mise à jour')).toBeTruthy()
    expect(screen.getByText('Prête')).toBeTruthy()
    expect(screen.getByText('Échec')).toBeTruthy()
    expect(screen.getByText('disk full')).toBeTruthy()
    expect(screen.getByText('1 / 3')).toBeTruthy()
    expect(screen.getByText('2 Mo', { exact: false })).toBeTruthy()
  })

  it('starts a named backup', async () => {
    setup([])
    let body: unknown
    server.use(
      http.post(`${API}${BACKUPS}`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(backup({ state: 'creating' }), { status: 202 })
      }),
    )
    const { user } = await renderApp(BACKUPS)

    await user.click(await screen.findByRole('button', { name: 'Nouvelle sauvegarde' }))
    await user.type(await screen.findByLabelText('Nom (facultatif)'), 'Avant la mise à jour')
    await user.click(screen.getByRole('button', { name: 'Sauvegarder' }))

    await waitFor(() => expect(body).toEqual({ name: 'Avant la mise à jour' }))
    expect(await screen.findByText('Sauvegarde lancée.')).toBeTruthy()
  })

  it('erases everything before restoring only once the name is typed', async () => {
    setup()
    let body: unknown
    server.use(
      http.post(`${API}${BACKUPS}/${READY_ID}/restore`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 202 })
      }),
    )
    const { user } = await renderApp(BACKUPS)

    await openActions(user, 'Avant la mise à jour')
    await user.click(await screen.findByRole('menuitem', { name: 'Restaurer' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('radio', { name: /Tout effacer/ }))
    const confirm = within(dialog).getByRole('button', { name: 'Tout effacer et restaurer' })
    expect(confirm.hasAttribute('disabled')).toBe(true)
    await user.type(within(dialog).getByLabelText('Tapez « Survie » pour confirmer'), 'Survie')
    await user.click(confirm)

    await waitFor(() => expect(body).toEqual({ mode: 'clean' }))
  })

  it('deletes a backup after confirmation', async () => {
    setup()
    let deleted = false
    server.use(
      http.delete(`${API}${BACKUPS}/${FAILED_ID}`, () => {
        deleted = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(BACKUPS)

    await openActions(user, 'Ratée')
    await user.click(await screen.findByRole('menuitem', { name: 'Supprimer' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(deleted).toBe(false)
    await user.click(within(dialog).getByRole('button', { name: 'Supprimer définitivement' }))

    await waitFor(() => expect(deleted).toBe(true))
  })

  it('downloads through a form that carries the ticket in its body', async () => {
    setup()
    server.use(
      http.post(`${API}${BACKUPS}/${READY_ID}/download/ticket`, () =>
        HttpResponse.json({ ticket: 'kct_secret', expires_in_secs: 30 }, { status: 201 }),
      ),
    )
    const submitted: HTMLFormElement[] = []
    const submit = vi.spyOn(HTMLFormElement.prototype, 'submit').mockImplementation(function (this: HTMLFormElement) {
      submitted.push(this.cloneNode(true) as HTMLFormElement)
    })
    const { user } = await renderApp(BACKUPS)

    await openActions(user, 'Avant la mise à jour')
    await user.click(await screen.findByRole('menuitem', { name: 'Télécharger' }))

    await waitFor(() => expect(submitted).toHaveLength(1))
    const form = submitted[0]!
    expect(form.method).toBe('post')
    expect(new URL(form.action).pathname).toBe(`/api/admin/v1${BACKUPS}/${READY_ID}/download`)
    expect(new URL(form.action).search).toBe('')
    expect((form.elements.namedItem('ticket') as HTMLInputElement).value).toBe('kct_secret')
    submit.mockRestore()
  })

  it('offers no action without the backup permissions', async () => {
    setup()
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read', 'servers.backups.read'] }),
      ),
    )
    await renderApp(BACKUPS)

    expect(await screen.findByText('Avant la mise à jour')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Nouvelle sauvegarde' })).toBeNull()
    expect(screen.queryByRole('button', { name: /Actions sur/ })).toBeNull()
  })

  it('shows the restoring state', async () => {
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers`, () => HttpResponse.json([serverDto({ state: 'restoring' })])),
    )
    await renderApp('/servers')

    expect(await screen.findByText('Restauration')).toBeTruthy()
  })
})
