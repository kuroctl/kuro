// Rôles (ADR-0007) : liste, éditeur (couleur, permissions par catégorie), rôle système.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { iam, role, SUPER_ADMIN_ID, superAdmin, SUPPORT_ID } from '../support/iam'

const MODERATION_ID = '01a17f00-0000-7000-8000-0000000000b2'

/** Noms des rôles, dans l'ordre des lignes. */
function names() {
  return screen
    .getAllByRole('row')
    .slice(1)
    .map((row) => within(row).getAllByRole('cell')[0]?.textContent?.replace('Système', '').trim())
}

describe('roles', () => {
  it('lists the roles with their members and permissions', async () => {
    server.use(...signedIn(), ...iam())
    await renderApp('/users/roles')

    expect(await screen.findByText('support')).toBeTruthy()
    expect(screen.getByText('Système')).toBeTruthy()
    expect(screen.getByText('Toutes')).toBeTruthy()
  })

  it('searches by name or description, sorts and opens a role', async () => {
    server.use(...signedIn(), ...iam())
    server.use(
      http.get(`${API}/roles`, () =>
        HttpResponse.json([
          superAdmin,
          role({ members: 2 }),
          role({ id: MODERATION_ID, name: 'moderation', description: 'Console et redémarrages.', members: 4, permissions: ['servers.power'] }),
        ]),
      ),
    )
    const { user, router } = await renderApp('/users/roles')

    await screen.findByText('moderation')
    expect(names()).toEqual(['moderation', 'super-admin', 'support'])

    const members = screen.getAllByRole('columnheader').find((cell) => cell.textContent?.includes('Membres'))!
    await user.click(within(members).getByRole('button'))
    await user.click(within(members).getByRole('button'))
    await waitFor(() => expect(names()).toEqual(['moderation', 'support', 'super-admin']))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'members', order: 'desc' })

    await user.type(screen.getByRole('searchbox', { name: 'Rechercher par nom ou description' }), 'journal')
    await waitFor(() => expect(names()).toEqual(['support']))
    expect(router.currentRoute.value.query.q).toBe('journal')

    await user.click(screen.getByText('support'))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/users/roles/${SUPPORT_ID}`))
  })

  it('creates a role with a palette color and translated permissions', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...iam(),
      http.post(`${API}/roles`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(role({ id: SUPPORT_ID, name: 'moderation' }), { status: 201 })
      }),
    )
    const { user, router } = await renderApp('/users/roles/new')

    await user.type(await screen.findByLabelText('Nom'), 'moderation')
    await user.click(screen.getByRole('radio', { name: '#22c55e' }))
    expect(await screen.findByText('Lister les serveurs, détail, état, statistiques')).toBeTruthy()
    await user.click(screen.getByRole('checkbox', { name: 'Toute la catégorie Serveurs' }))
    await user.click(screen.getByRole('button', { name: 'Créer le rôle' }))

    await waitFor(() =>
      expect(body).toEqual({ name: 'moderation', description: '', color: '#22c55e', permissions: ['servers.read', 'servers.power'] }),
    )
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/users/roles/${SUPPORT_ID}`))
  })

  it('never grants a permission to come', async () => {
    server.use(...signedIn(), ...iam())
    await renderApp('/users/roles/new')

    const transfer = (await screen.findByText('Déplacer un serveur vers un autre nœud')).closest('label')!
    expect(within(transfer).getByRole('checkbox').hasAttribute('disabled')).toBe(true)
    expect(within(transfer).getByText('Bientôt')).toBeTruthy()
  })

  it('sends only what changed', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...iam(),
      http.patch(`${API}/roles/${SUPPORT_ID}`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(role())
      }),
    )
    const { user } = await renderApp(`/users/roles/${SUPPORT_ID}`)

    const hex = await screen.findByLabelText('Couleur hexadécimale')
    await user.clear(hex)
    await user.type(hex, '#ABCDEF')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ color: '#abcdef' }))
  })

  it('shows the system role read only', async () => {
    server.use(...signedIn(), ...iam())
    await renderApp(`/users/roles/${SUPER_ADMIN_ID}`)

    expect(await screen.findByText(/Rôle livré par Kuro/)).toBeTruthy()
    expect(((await screen.findByLabelText('Nom')) as HTMLInputElement).disabled).toBe(true)
    expect(screen.queryByRole('button', { name: 'Enregistrer' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Supprimer' })).toBeNull()
  })

  it('greys out the permissions the admin does not hold', async () => {
    server.use(...signedIn(), ...iam())
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['iam.roles.read', 'iam.roles.manage', 'servers.read'] }),
      ),
    )
    await renderApp('/users/roles/new')

    const power = (await screen.findByText('Démarrer, arrêter, redémarrer, tuer')).closest('label')!
    expect(within(power).getByRole('checkbox').hasAttribute('disabled')).toBe(true)
    expect(within(power).getByText('Vous ne la détenez pas')).toBeTruthy()
  })

  it('warns before deleting a role that still has members', async () => {
    let deleted = false
    server.use(
      ...signedIn(),
      ...iam(),
      http.delete(`${API}/roles/${SUPPORT_ID}`, () => {
        deleted = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user, router } = await renderApp(`/users/roles/${SUPPORT_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Supprimer' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText('1 admin(s) perdront ce rôle et les permissions qu’il accorde.')).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Supprimer le rôle' }))

    await waitFor(() => expect(deleted).toBe(true))
    await waitFor(() => expect(router.currentRoute.value.path).toBe('/users/roles'))
  })

  it('translates an escalation refusal', async () => {
    server.use(
      ...signedIn(),
      ...iam(),
      http.patch(`${API}/roles/${SUPPORT_ID}`, () => problem(403, 'cannot_grant_permissions_you_lack')),
    )
    const { user } = await renderApp(`/users/roles/${SUPPORT_ID}`)

    await user.click(await screen.findByRole('checkbox', { name: 'Toute la catégorie Serveurs' }))
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    expect(await screen.findByText('Vous ne pouvez pas accorder des permissions que vous n’avez pas.')).toBeTruthy()
  })
})
