// Première mise en route (ADR-0045) : tout mène à l'assistant tant que l'API n'a aucun compte ;
// jeton, compte, nom du projet envoyés en une fois ; champ refusé ramené à son écran ; puis la
// connexion. Mise en route faite : l'assistant renvoie à la connexion, qui affiche le projet.
import { screen, waitFor } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { accessToken, anonymous, API, problem, server } from '../support/api'
import { renderApp } from '../support/app'

// Le QR code passe par un canvas, absent de jsdom.
vi.mock('qrcode', () => ({ default: { toDataURL: async () => 'data:image/png;base64,AAAA' } }))

const needed = () => http.get('/api/setup', () => HttpResponse.json({ needed: true, project_name: null }))

describe('first-run setup', () => {
  it('leads to the wizard, creates the super admin and the project, then signs in', async () => {
    let body: unknown
    let done = false
    server.use(
      ...anonymous(),
      http.get('/api/setup', () => HttpResponse.json({ needed: !done, project_name: done ? 'Studio Nuage' : null })),
      http.post('/api/setup', async ({ request }) => {
        body = await request.json()
        done = true
        return HttpResponse.json({ user_id: '01a1786d-d121-71d1-a067-40612b7a2c50', email: 'owner@example.com' }, { status: 201 })
      }),
      http.post(`${API}/auth/login`, () => HttpResponse.json({ status: 'authenticated', ...accessToken() })),
    )
    const { user, router } = await renderApp('/servers')
    expect(router.currentRoute.value.path).toBe('/setup')

    expect(await screen.findByText('Bienvenue sur Kuro')).toBeTruthy()
    await user.type(screen.getByLabelText('Jeton de mise en route'), 'kuro_setup_abc')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))

    await user.type(await screen.findByLabelText('Nom affiché'), 'Owner')
    await user.type(screen.getByLabelText('Adresse e-mail'), 'owner@example.com')
    await user.type(screen.getByLabelText('Mot de passe'), 'a long enough password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'a long enough password')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))

    await user.type(await screen.findByLabelText('Nom du projet (facultatif)'), 'Studio Nuage')
    await user.click(screen.getByRole('button', { name: 'Créer le compte' }))

    await waitFor(() =>
      expect(body).toEqual({
        token: 'kuro_setup_abc',
        email: 'owner@example.com',
        display_name: 'Owner',
        password: 'a long enough password',
        project_name: 'Studio Nuage',
      }),
    )
    // Puis la connexion habituelle, sur place.
    expect(await screen.findByRole('button', { name: 'Se connecter' })).toBeTruthy()
  })

  it('brings a refused field back to its screen', async () => {
    server.use(
      ...anonymous(),
      needed(),
      http.post('/api/setup', () => problem(403, 'invalid_token')),
    )
    const { user } = await renderApp('/setup')

    await user.type(await screen.findByLabelText('Jeton de mise en route'), 'kuro_setup_wrong')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    await user.type(await screen.findByLabelText('Nom affiché'), 'Owner')
    await user.type(screen.getByLabelText('Adresse e-mail'), 'owner@example.com')
    await user.type(screen.getByLabelText('Mot de passe'), 'a long enough password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'a long enough password')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    await user.click(await screen.findByRole('button', { name: 'Créer le compte' }))

    // Un jeton refusé n'est pas une erreur de champ : toast, et retour à l'écran du jeton.
    expect(await screen.findByText('Bienvenue sur Kuro')).toBeTruthy()
  })

  it('refuses mismatched passwords before sending anything', async () => {
    server.use(...anonymous(), needed())
    const { user } = await renderApp('/setup')

    await user.type(await screen.findByLabelText('Jeton de mise en route'), 'kuro_setup_abc')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))
    await user.type(await screen.findByLabelText('Nom affiché'), 'Owner')
    await user.type(screen.getByLabelText('Adresse e-mail'), 'owner@example.com')
    await user.type(screen.getByLabelText('Mot de passe'), 'a long enough password')
    await user.type(screen.getByLabelText('Confirmez le mot de passe'), 'another password 12')
    await user.click(screen.getByRole('button', { name: 'Continuer' }))

    expect(await screen.findByText('Les deux mots de passe ne correspondent pas.')).toBeTruthy()
  })

  it('sends a visitor of a configured API from the wizard to the sign-in page, named after the project', async () => {
    server.use(
      ...anonymous(),
      http.get('/api/setup', () => HttpResponse.json({ needed: false, project_name: 'Studio Nuage' })),
    )
    const { router } = await renderApp('/setup')

    expect(router.currentRoute.value.path).toBe('/login')
    expect(await screen.findByText('Studio Nuage')).toBeTruthy()
  })

  it('lets a visitor switch the language before signing in or setting up', async () => {
    server.use(...anonymous(), needed())
    const { user } = await renderApp('/setup')

    expect(await screen.findByText('Bienvenue sur Kuro')).toBeTruthy()
    await user.click(screen.getByRole('combobox', { name: 'Langue' }))
    await user.click(await screen.findByRole('option', { name: 'English' }))
    expect(await screen.findByText('Welcome to Kuro')).toBeTruthy()
    // Le sélecteur garde chaque langue dans sa propre langue.
    expect(screen.getByRole('combobox', { name: 'Language' }).textContent).toContain('English')
  })
})
