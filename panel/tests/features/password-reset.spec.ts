// Mot de passe oublié (ADR-0049) : lien depuis la connexion, demande qui ne révèle pas les
// comptes, choix du nouveau mot de passe depuis le lien reçu, lien refusé.
import { screen, waitFor } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { accessToken, anonymous, API, problem, profile, server } from '../support/api'
import { renderApp } from '../support/app'

describe('forgotten password', () => {
  it('requests a link from the login page', async () => {
    let body: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/password/forgot`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 202 })
      }),
    )
    const { user, router } = await renderApp('/login')

    await user.click(await screen.findByRole('link', { name: 'Mot de passe oublié ?' }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe('/password/forgot'))
    await user.type(await screen.findByLabelText('Adresse e-mail'), 'admin@example.com')
    await user.click(screen.getByRole('button', { name: 'Envoyer le lien' }))

    expect(await screen.findByText('Vérifiez votre boîte de réception')).toBeTruthy()
    expect(screen.getByText(/Si admin@example.com est l’adresse d’un compte actif/)).toBeTruthy()
    expect(body).toEqual({ email: 'admin@example.com' })
    await user.click(screen.getByRole('link', { name: 'Retour à la connexion' }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe('/login'))
  })

  it('sets the new password from the emailed link, then signs in', async () => {
    let body: unknown
    server.use(
      ...anonymous(),
      http.post(`${API}/auth/password/reset`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ status: 'authenticated', ...accessToken() })
      }),
      http.get(`${API}/me`, () => HttpResponse.json(profile)),
    )
    const { user, router } = await renderApp('/password/reset/kuro_pwd_abc')

    expect(await screen.findByText(/Choisissez votre nouveau mot de passe/)).toBeTruthy()
    await user.type(screen.getByLabelText('Nouveau mot de passe'), 'my very own password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'my very own pass')
    await user.click(screen.getByRole('button', { name: 'Changer le mot de passe' }))
    expect(await screen.findByText('Les deux mots de passe ne correspondent pas.')).toBeTruthy()
    expect(body).toBeUndefined()

    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'word')
    await user.click(screen.getByRole('button', { name: 'Changer le mot de passe' }))

    await waitFor(() => expect(body).toEqual({ token: 'kuro_pwd_abc', password: 'my very own password' }))
    await waitFor(() => expect(router.currentRoute.value.path).toBe('/servers'))
  })

  it('offers a new link when the link is no longer valid', async () => {
    server.use(...anonymous(), http.post(`${API}/auth/password/reset`, () => problem(403, 'invalid_token')))
    const { user } = await renderApp('/password/reset/kuro_pwd_old')

    await user.type(await screen.findByLabelText('Nouveau mot de passe'), 'my very own password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'my very own password')
    await user.click(screen.getByRole('button', { name: 'Changer le mot de passe' }))

    expect(await screen.findByText('Token invalide, expiré ou déjà utilisé.')).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Demander un nouveau lien' })).toBeTruthy()
  })
})
