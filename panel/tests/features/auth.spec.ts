// Connexion admin : reprise de session, identifiants, TOTP, erreurs, déconnexion.
import type { UserEvent } from '@testing-library/user-event'
import { screen, waitFor } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { accessToken, anonymous, API, logout, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

// Le QR code passe par un canvas, absent de jsdom.
vi.mock('qrcode', () => ({ default: { toDataURL: async () => 'data:image/png;base64,AAAA' } }))

async function fillCredentials(user: UserEvent) {
  await user.type(screen.getByLabelText('Adresse e-mail'), 'admin@example.com')
  await user.type(screen.getByLabelText('Mot de passe'), 'correct horse battery')
  await user.click(screen.getByRole('button', { name: 'Se connecter' }))
}

describe('admin sign-in', () => {
  it('redirects an anonymous visitor to the login page', async () => {
    server.use(...anonymous())
    const { router } = await renderApp('/account')

    expect(router.currentRoute.value.path).toBe('/login')
    expect(router.currentRoute.value.query.redirect).toBe('/account')
    expect(await screen.findByRole('button', { name: 'Se connecter' })).toBeTruthy()
  })

  it('resumes the session from the refresh cookie with a single refresh', async () => {
    // L'API fait tourner le cookie : un second rafraîchissement concurrent serait refusé.
    let refreshes = 0
    const anonymousCalls: string[] = []
    server.use(
      http.post(`${API}/auth/refresh`, () => {
        refreshes += 1
        return HttpResponse.json(accessToken())
      }),
      http.get(`${API}/me`, ({ request }) => {
        if (!request.headers.get('authorization')) anonymousCalls.push(request.url)
        return HttpResponse.json(profile)
      }),
    )
    const { router } = await renderApp('/account')

    expect(router.currentRoute.value.path).toBe('/account')
    expect(await screen.findByText('admin@example.com')).toBeTruthy()
    expect(refreshes).toBe(1)
    expect(anonymousCalls).toEqual([])
  })

  it('signs in and returns to the requested page', async () => {
    let body: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/login`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ status: 'authenticated', ...accessToken() })
      }),
      http.get(`${API}/me`, ({ request }) =>
        request.headers.get('authorization') === 'Bearer token-1'
          ? HttpResponse.json(profile)
          : problem(401, 'unauthorized'),
      ),
    )
    const { router, user } = await renderApp('/account')
    await fillCredentials(user)

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/account'))
    expect(body).toEqual({ email: 'admin@example.com', password: 'correct horse battery' })
    expect(await screen.findByText('admin@example.com')).toBeTruthy()
  })

  it('shows invalid credentials as a translated toast with the request reference', async () => {
    server.use(...anonymous(), http.post(`${API}/auth/login`, () => problem(401, 'invalid_credentials')))
    const { router, user } = await renderApp('/login')
    await fillCredentials(user)

    expect(await screen.findByText('Identifiants invalides.')).toBeTruthy()
    expect(screen.getByText('Référence : 01234567')).toBeTruthy()
    expect(router.currentRoute.value.path).toBe('/login')
  })

  it('shows field errors under their field instead of a toast', async () => {
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/login`, () =>
        problem(422, 'validation_failed', { fields: [{ field: 'email', code: 'invalid_email' }] }),
      ),
    )
    const { user } = await renderApp('/login')
    await fillCredentials(user)

    expect(await screen.findByText('Adresse e-mail invalide.')).toBeTruthy()
    expect(screen.queryByText('Certains champs sont invalides.')).toBeNull()
  })

  it('asks for the TOTP code when two-step verification is enabled', async () => {
    let verified: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/login`, () =>
        HttpResponse.json({ status: 'totp_required', challenge: 'challenge-1', expires_in: 300 }),
      ),
      http.post(`${API}/auth/totp/verify`, async ({ request }) => {
        verified = await request.json()
        return HttpResponse.json(accessToken())
      }),
      http.get(`${API}/me`, () => HttpResponse.json(profile)),
    )
    const { router, user } = await renderApp('/login')
    await fillCredentials(user)

    await user.type(await screen.findByLabelText('Code'), '123456')
    await user.click(screen.getByRole('button', { name: 'Vérifier' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers'))
    expect(verified).toEqual({ challenge: 'challenge-1', code: '123456' })
  })

  it('walks a new admin through the TOTP setup', async () => {
    let confirmed: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/login`, () =>
        HttpResponse.json({ status: 'totp_setup_required', challenge: 'challenge-2', expires_in: 300 }),
      ),
      http.post(`${API}/auth/totp/setup`, () =>
        HttpResponse.json({ secret: 'JBSWY3DPEHPK3PXP', otpauth_uri: 'otpauth://totp/Kuro:admin' }),
      ),
      http.post(`${API}/auth/totp/confirm`, async ({ request }) => {
        confirmed = await request.json()
        return HttpResponse.json(accessToken())
      }),
      http.get(`${API}/me`, () => HttpResponse.json(profile)),
    )
    const { router, user } = await renderApp('/login')
    await fillCredentials(user)

    expect(await screen.findByAltText('QR code de configuration')).toBeTruthy()
    expect(screen.getByText('JBSWY3DPEHPK3PXP')).toBeTruthy()
    await user.type(screen.getByLabelText('Code'), '654321')
    await user.click(screen.getByRole('button', { name: 'Activer' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers'))
    expect(confirmed).toEqual({ challenge: 'challenge-2', code: '654321' })
  })

  it('makes an admin replace a password chosen by someone else before the TOTP', async () => {
    let changed: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/login`, () =>
        HttpResponse.json({ status: 'password_change_required', challenge: 'challenge-p', expires_in: 300 }),
      ),
      http.post(`${API}/auth/password`, async ({ request }) => {
        changed = await request.json()
        return HttpResponse.json({ status: 'totp_required', challenge: 'challenge-t', expires_in: 300 })
      }),
      http.post(`${API}/auth/totp/verify`, () => HttpResponse.json(accessToken())),
      http.get(`${API}/me`, () => HttpResponse.json(profile)),
    )
    const { router, user } = await renderApp('/login')
    await fillCredentials(user)

    expect(await screen.findByText('Choisissez votre mot de passe')).toBeTruthy()
    await user.type(screen.getByLabelText('Nouveau mot de passe'), 'my very own password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'my very own pass')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    expect(await screen.findByText('Les deux mots de passe ne correspondent pas.')).toBeTruthy()
    expect(changed).toBeUndefined()

    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'word')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    await user.type(await screen.findByLabelText('Code'), '123456')
    await user.click(screen.getByRole('button', { name: 'Vérifier' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers'))
    expect(changed).toEqual({ challenge: 'challenge-p', new_password: 'my very own password' })
  })

  it('shows a refused new password under its field', async () => {
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/login`, () =>
        HttpResponse.json({ status: 'password_change_required', challenge: 'challenge-p', expires_in: 300 }),
      ),
      http.post(`${API}/auth/password`, () =>
        problem(422, 'password_unchanged', { fields: [{ field: 'new_password', code: 'password_unchanged', params: {} }] }),
      ),
    )
    const { user } = await renderApp('/login')
    await fillCredentials(user)

    await user.type(await screen.findByLabelText('Nouveau mot de passe'), 'temporary password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'temporary password')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))

    expect(await screen.findByText('Choisissez un mot de passe différent de celui reçu.')).toBeTruthy()
  })

  it('signs out from the account menu', async () => {
    server.use(...signedIn(), logout())
    const { router, user } = await renderApp('/servers')

    await user.click(await screen.findByRole('button', { name: 'Mon compte' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Se déconnecter' }))

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/login'))
  })
})

