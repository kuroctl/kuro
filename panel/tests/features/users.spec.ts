// Comptes admin (ADR-0007) : liste, création, rôles, identifiants, désactivation.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { ADMIN_ID, admin, iam, me, SUPER_ADMIN_ID, SUPPORT_ID } from '../support/iam'

const BOB_ID = '01a17f00-0000-7000-8000-0000000000c2'
const CAROL_ID = '01a17f00-0000-7000-8000-0000000000c3'
const team = [
  { ...me, last_login_at: '2026-10-20T09:00:00Z' },
  admin(),
  admin({ id: BOB_ID, display_name: 'Bob', email: 'bob@example.com', last_login_at: '2026-10-28T09:00:00Z' }),
  admin({ id: CAROL_ID, display_name: 'Carol', email: 'carol@example.com', roles: [], disabled: true, last_login_at: null }),
]

/** Noms affichés, dans l'ordre des lignes. */
function names() {
  return screen
    .getAllByRole('row')
    .slice(1)
    .map((row) => within(row).getAllByRole('cell')[0]?.querySelector('.font-medium')?.firstChild?.textContent?.trim())
}

function facet(name: string) {
  return screen.getAllByRole('button', { name }).find((button) => !button.closest('th'))!
}

function header(name: string) {
  return screen.getAllByRole('columnheader').find((cell) => cell.textContent?.includes(name))!
}

describe('admin accounts', () => {
  it('lists the admins with their roles, TOTP and state, and finds them by name or email', async () => {
    server.use(...signedIn(), ...iam())
    const { user } = await renderApp('/users')

    expect(await screen.findByText('Alice')).toBeTruthy()
    expect(screen.getByText('alice@example.com')).toBeTruthy()
    expect(screen.getByText('support')).toBeTruthy()
    expect(screen.getByText('Vous')).toBeTruthy()
    await user.type(screen.getByRole('searchbox', { name: 'Rechercher par nom ou e-mail' }), 'alice@')

    await waitFor(() => expect(screen.queryByText('admin@example.com')).toBeNull())
    expect(screen.getByText('Alice')).toBeTruthy()
  })

  it('filters by role and state, counting with the other filter applied, and keeps the view in the URL', async () => {
    server.use(...signedIn(), ...iam())
    server.use(http.get(`${API}/users`, () => HttpResponse.json(team)))
    const { user, router } = await renderApp('/users')

    await screen.findByText('Carol')
    await user.click(facet('État'))
    let popover = await screen.findByRole('dialog')
    expect(within(popover).getByText('Actif').parentElement?.textContent).toContain('3')
    expect(within(popover).getByText('Désactivé').parentElement?.textContent).toContain('1')
    await user.click(within(popover).getByText('Actif'))
    await waitFor(() => expect(names()).toEqual(['Admin', 'Alice', 'Bob']))
    expect(router.currentRoute.value.query.state).toBe('active')
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())

    await user.click(facet('Rôles'))
    popover = await screen.findByRole('dialog')
    // Compté parmi les comptes actifs : Carol, désactivée, n'y figure pas.
    expect(within(popover).getByText('support').parentElement?.textContent).toContain('2')
    expect(within(popover).getByText('Aucun rôle').parentElement?.textContent).toContain('0')
    await user.click(within(popover).getByText('support'))
    await waitFor(() => expect(names()).toEqual(['Alice', 'Bob']))
    expect(router.currentRoute.value.query.role).toBe(SUPPORT_ID)
  })

  it('sorts by a column and opens an admin from its row', async () => {
    server.use(...signedIn(), ...iam())
    server.use(http.get(`${API}/users`, () => HttpResponse.json(team)))
    const { user, router } = await renderApp('/users')

    await screen.findByText('Carol')
    expect(names()).toEqual(['Admin', 'Alice', 'Bob', 'Carol'])
    await user.click(within(header('Nom')).getByRole('button'))
    await waitFor(() => expect(names()).toEqual(['Carol', 'Bob', 'Alice', 'Admin']))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'name', order: 'desc' })

    // Jamais connecté : en dernier, dans les deux sens.
    await user.click(within(header('Dernière connexion')).getByRole('button'))
    await waitFor(() => expect(names()).toEqual(['Admin', 'Alice', 'Bob', 'Carol']))
    await user.click(within(header('Dernière connexion')).getByRole('button'))
    await waitFor(() => expect(names()).toEqual(['Bob', 'Alice', 'Admin', 'Carol']))
    expect(router.currentRoute.value.query).toMatchObject({ sort: 'lastLogin', order: 'desc' })

    await user.click(screen.getByText('Bob'))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/users/${BOB_ID}`))
  })

  it('creates an admin with a generated password, then grants the roles', async () => {
    let created: Record<string, unknown> | undefined
    let roles: unknown
    server.use(
      ...signedIn(),
      ...iam(),
      http.post(`${API}/users`, async ({ request }) => {
        created = (await request.json()) as Record<string, unknown>
        return HttpResponse.json(admin({ id: ADMIN_ID, display_name: 'Bob', email: 'bob@example.com', roles: [] }), { status: 201 })
      }),
      http.put(`${API}/users/${ADMIN_ID}/roles`, async ({ request }) => {
        roles = await request.json()
        return HttpResponse.json(admin())
      }),
    )
    const { user, router } = await renderApp('/users')

    await user.click(await screen.findByRole('button', { name: 'Nouvel admin' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Bob')
    await user.type(within(dialog).getByLabelText('Adresse e-mail'), 'bob@example.com')
    await user.click(within(dialog).getByRole('button', { name: 'Générer un mot de passe' }))
    await user.click(within(dialog).getByRole('checkbox', { name: 'support' }))
    await user.click(within(dialog).getByRole('button', { name: 'Créer le compte' }))

    await waitFor(() => expect(roles).toEqual({ role_ids: [SUPPORT_ID] }))
    expect(created).toMatchObject({ display_name: 'Bob', email: 'bob@example.com' })
    expect(String(created?.password)).toMatch(/^[A-Za-z0-9_-]{20}$/)
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/users/${ADMIN_ID}`))
  })

  it('shows a refused field under it', async () => {
    server.use(
      ...signedIn(),
      ...iam(),
      http.post(`${API}/users`, () =>
        problem(422, 'email_taken', { fields: [{ field: 'email', code: 'email_taken', params: {} }] }),
      ),
    )
    const { user } = await renderApp('/users')

    await user.click(await screen.findByRole('button', { name: 'Nouvel admin' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Bob')
    await user.type(within(dialog).getByLabelText('Adresse e-mail'), 'alice@example.com')
    await user.type(within(dialog).getByLabelText('Mot de passe'), 'long enough password')
    await user.click(within(dialog).getByRole('button', { name: 'Créer le compte' }))

    expect(await within(dialog).findByText('Cette adresse e-mail est déjà utilisée.')).toBeTruthy()
  })

  it('greys out a role that grants permissions the admin does not hold', async () => {
    server.use(...signedIn(), ...iam())
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({
          ...profile,
          all_permissions: false,
          permissions: ['iam.users.read', 'iam.roles.read', 'iam.roles.assign', 'servers.read', 'audit.read'],
        }),
      ),
    )
    await renderApp(`/users/${ADMIN_ID}`)

    const support = await screen.findByRole('checkbox', { name: 'support' })
    const everything = screen.getByRole('checkbox', { name: 'super-admin' })
    expect(support.hasAttribute('disabled')).toBe(false)
    expect(everything.hasAttribute('disabled')).toBe(true)
  })

  it('saves the chosen roles of an admin', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...iam(),
      http.put(`${API}/users/${ADMIN_ID}/roles`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(admin())
      }),
    )
    const { user } = await renderApp(`/users/${ADMIN_ID}`)

    await user.click(await screen.findByRole('checkbox', { name: 'super-admin' }))
    const card = screen.getByText('Rôles', { selector: '[data-slot="card-title"], h3, div' }).closest('[data-slot="card"]') as HTMLElement
    await user.click(within(card ?? document.body).getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ role_ids: [SUPPORT_ID, SUPER_ADMIN_ID] }))
  })

  it('cannot change its own roles', async () => {
    server.use(
      ...signedIn(),
      ...iam(),
      http.get(`${API}/users/${profile.user.id}`, () =>
        HttpResponse.json({ ...profile.user, last_login_at: null }),
      ),
    )
    await renderApp(`/users/${profile.user.id}`)

    expect(await screen.findByText('Vous ne pouvez pas modifier vos propres rôles.')).toBeTruthy()
    expect((await screen.findByRole('checkbox', { name: 'support' })).hasAttribute('disabled')).toBe(true)
    expect(screen.getByRole('button', { name: 'Supprimer' }).hasAttribute('disabled')).toBe(true)
  })

  it('resets the password with a new one', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...iam(),
      http.put(`${API}/users/${ADMIN_ID}/password`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(`/users/${ADMIN_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Réinitialiser le mot de passe' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Mot de passe'), 'another long password')
    await user.click(within(dialog).getByRole('button', { name: 'Réinitialiser le mot de passe' }))

    await waitFor(() => expect(body).toEqual({ password: 'another long password' }))
    expect(await screen.findByText('Mot de passe de Alice réinitialisé.')).toBeTruthy()
  })

  it('flags an admin who must replace a password chosen for them', async () => {
    server.use(...signedIn(), ...iam())
    server.use(http.get(`${API}/users/${ADMIN_ID}`, () => HttpResponse.json(admin({ password_change_required: true }))))
    await renderApp(`/users/${ADMIN_ID}`)

    expect(await screen.findByText('Doit changer son mot de passe')).toBeTruthy()
  })

  it('disables an account after confirmation and explains a refusal', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      ...iam(),
      http.patch(`${API}/users/${ADMIN_ID}`, async ({ request }) => {
        body = await request.json()
        return problem(409, 'last_super_admin')
      }),
    )
    const { user } = await renderApp(`/users/${ADMIN_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Désactiver' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(body).toBeUndefined()
    await user.click(within(dialog).getByRole('button', { name: 'Désactiver' }))

    await waitFor(() => expect(body).toEqual({ disabled: true }))
    expect(await screen.findByText('Le dernier super-admin actif doit rester en place.')).toBeTruthy()
  })

  it('offers nothing to change without the permissions', async () => {
    server.use(...signedIn(), ...iam())
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['iam.users.read'] })),
    )
    await renderApp(`/users/${ADMIN_ID}`)

    expect(((await screen.findByLabelText('Nom')) as HTMLInputElement).disabled).toBe(true)
    expect(screen.queryByRole('button', { name: 'Enregistrer' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Réinitialiser le mot de passe' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Supprimer' })).toBeNull()
  })
})
