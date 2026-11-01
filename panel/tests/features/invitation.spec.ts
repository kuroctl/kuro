// Invitations des admins (ADR-0046) : le lien reçu mène à un choix de mot de passe hors coque,
// puis à la session ; un jeton refusé est dit ; un admin se crée en l'invitant quand le SMTP est
// réglé, et une invitation se renvoie depuis sa fiche.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { accessToken, anonymous, API, mailNotConfigured, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { ADMIN_ID, admin, iam } from '../support/iam'

// Le QR code passe par un canvas, absent de jsdom.
vi.mock('qrcode', () => ({ default: { toDataURL: async () => 'data:image/png;base64,AAAA' } }))

const mailConfigured = { ...mailNotConfigured, configured: true, host: 'smtp.example.com', port: 587, security: 'starttls', from_address: 'kuro@example.com' }

describe('admin invitations', () => {
  it('activates the account from the link, then opens the session', async () => {
    let body: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/invitation`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ status: 'authenticated', ...accessToken() })
      }),
      http.get(`${API}/me`, () => HttpResponse.json({ user: admin(), permissions: [], all_permissions: true, totp_required: false, recovery_codes_remaining: 0 })),
    )
    const { user, router } = await renderApp('/invitation/kuro_inv_abc')

    expect(await screen.findByText('Bienvenue')).toBeTruthy()
    await user.type(screen.getByLabelText('Nouveau mot de passe'), 'short')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'short')
    await user.click(screen.getByRole('button', { name: 'Activer mon compte' }))
    expect(await screen.findByText('Le mot de passe doit contenir au moins 12 caractères.')).toBeTruthy()
    expect(body).toBeUndefined()

    await user.clear(screen.getByLabelText('Nouveau mot de passe'))
    await user.clear(screen.getByLabelText('Confirmez le mot de passe'))
    await user.type(screen.getByLabelText('Nouveau mot de passe'), 'my very own password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'my very own password')
    await user.click(screen.getByRole('button', { name: 'Activer mon compte' }))

    await waitFor(() => expect(body).toEqual({ token: 'kuro_inv_abc', password: 'my very own password' }))
    // L'accueil mène à l'infrastructure.
    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers'))
  })

  it('tells when the link is no longer valid', async () => {
    server.use(...anonymous(), http.post(`${API}/auth/invitation`, () => problem(403, 'invalid_token')))
    const { user } = await renderApp('/invitation/kuro_inv_old')

    await user.type(await screen.findByLabelText('Nouveau mot de passe'), 'my very own password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'my very own password')
    await user.click(screen.getByRole('button', { name: 'Activer mon compte' }))

    expect(await screen.findByText('Token invalide, expiré ou déjà utilisé.')).toBeTruthy()
  })

  it('creates an admin by invitation when SMTP is configured, and resends it from the account', async () => {
    let created: unknown
    let reinvited = false
    // Le réglage par défaut de `signedIn()` dit « non configuré » : le nôtre passe devant.
    server.use(
      http.get(`${API}/settings/mail`, () => HttpResponse.json(mailConfigured)),
      http.get(`${API}/users/${ADMIN_ID}`, () => HttpResponse.json(admin({ display_name: 'Bob', email: 'bob@example.com', invitation_pending: true }))),
      ...signedIn(),
      ...iam(),
      http.post(`${API}/users`, async ({ request }) => {
        created = await request.json()
        return HttpResponse.json(admin({ id: ADMIN_ID, display_name: 'Bob', email: 'bob@example.com', roles: [], invitation_pending: true }), { status: 201 })
      }),
      http.post(`${API}/users/${ADMIN_ID}/invite`, () => {
        reinvited = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user, router } = await renderApp('/users')

    await user.click(await screen.findByRole('button', { name: 'Nouvel admin' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Bob')
    await user.type(within(dialog).getByLabelText('Adresse e-mail'), 'bob@example.com')
    // SMTP réglé : l'invitation est proposée par défaut, sans champ de mot de passe.
    expect(within(dialog).getByRole('switch', { name: 'Inviter par e-mail' }).getAttribute('aria-checked')).toBe('true')
    expect(within(dialog).queryByLabelText('Mot de passe')).toBeNull()
    await user.click(within(dialog).getByRole('button', { name: 'Créer et inviter' }))

    await waitFor(() => expect(created).toEqual({ display_name: 'Bob', email: 'bob@example.com', invite: true }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe(`/users/${ADMIN_ID}`))
    expect(await screen.findByText('Invitation en attente')).toBeTruthy()
    await user.click(await screen.findByRole('button', { name: 'Renvoyer l’invitation' }))
    await waitFor(() => expect(reinvited).toBe(true))
    expect(await screen.findByText('Invitation envoyée à Bob.')).toBeTruthy()
  })
})
