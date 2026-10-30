// Compte Steam des installations (ADR-0033) : enregistré avec son identifiant affiché, mot de
// passe jamais réaffiché, erreurs sous leur champ, retrait après confirmation ; page réservée à
// sa permission.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

const ACCOUNT = `${API}/settings/steam-account`
const PAGE = '/settings/steam-account'

function account(username: string | null): Schemas['SteamAccountDto'] {
  return { configured: !!username, username, updated_at: username ? '2026-10-29T10:00:00Z' : null }
}

describe('steam account', () => {
  it('saves the account, keeps the username and clears the password', async () => {
    let body: unknown
    let saved: string | null = null
    server.use(
      ...signedIn(),
      http.get(ACCOUNT, () => HttpResponse.json(account(saved))),
      http.put(ACCOUNT, async ({ request }) => {
        body = await request.json()
        saved = 'kuro_servers'
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(PAGE)

    expect(await screen.findByText('Aucun compte')).toBeTruthy()
    expect(screen.getByText('Désactivez Steam Guard sur ce compte : SteamCMD ne peut pas saisir de code.')).toBeTruthy()
    await user.type(screen.getByLabelText('Identifiant Steam'), ' kuro_servers ')
    await user.type(screen.getByLabelText('Mot de passe'), 'Pa ss!word')
    await user.click(screen.getByRole('button', { name: 'Enregistrer le compte' }))

    await waitFor(() => expect(body).toEqual({ username: 'kuro_servers', password: 'Pa ss!word' }))
    expect(await screen.findByText('Compte Steam enregistré : il servira à la prochaine installation.')).toBeTruthy()
    expect(await screen.findByText('Réglé')).toBeTruthy()
    expect(screen.getByLabelText('Identifiant Steam')).toHaveProperty('value', 'kuro_servers')
    expect(screen.getByLabelText('Nouveau mot de passe')).toHaveProperty('value', '')
  })

  it('shows a refused value under its field', async () => {
    server.use(
      ...signedIn(),
      http.get(ACCOUNT, () => HttpResponse.json(account(null))),
      http.put(ACCOUNT, () =>
        problem(422, 'validation_failed', { fields: [{ field: 'password', code: 'steam_account_password_invalid', params: {} }] }),
      ),
    )
    const { user } = await renderApp(PAGE)

    await user.type(await screen.findByLabelText('Identifiant Steam'), 'kuro_servers')
    await user.type(screen.getByLabelText('Mot de passe'), 'a"b')
    await user.click(screen.getByRole('button', { name: 'Enregistrer le compte' }))

    expect(await screen.findByText('64 caractères au plus, sans guillemet, barre oblique inverse ni caractère de contrôle.')).toBeTruthy()
  })

  it('removes the account after confirmation', async () => {
    let removed = false
    server.use(
      ...signedIn(),
      http.get(ACCOUNT, () => HttpResponse.json(account(removed ? null : 'kuro_servers'))),
      http.delete(ACCOUNT, () => {
        removed = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('button', { name: 'Retirer le compte' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: 'Retirer le compte' }))

    await waitFor(() => expect(removed).toBe(true))
    expect(await screen.findByText('Aucun compte')).toBeTruthy()
  })

  it('is out of reach without its permission', async () => {
    server.use(...signedIn())
    // Après `signedIn()` : dans un même appel, le premier handler de `/me` gagnerait.
    server.use(http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['system.settings.read'] })))
    await renderApp(PAGE)

    expect(await screen.findByText('Vous n’avez pas accès au compte Steam des installations.')).toBeTruthy()
    expect(screen.queryByRole('link', { name: 'Compte Steam' })).toBeNull()
  })
})
