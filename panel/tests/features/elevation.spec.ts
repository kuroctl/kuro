// Élévation de session (ADR-0050) : une action protégée refusée par `403 elevation_required`
// ouvre la confirmation d'identité commune (mot de passe, ou code TOTP), puis la requête est
// rejouée ; une confirmation abandonnée n'affiche aucune erreur.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { ADMIN_ID, iam } from '../support/iam'

/** Réinitialisation du mot de passe d'Alice, protégée : refusée tant que la session n'est pas élevée. */
function protectedReset(method: 'totp' | 'password') {
  const calls: unknown[] = []
  const elevations: unknown[] = []
  let elevated = false
  server.use(
    ...signedIn(),
    ...iam(),
    http.put(`${API}/users/${ADMIN_ID}/password`, async ({ request }) => {
      calls.push(await request.json())
      if (!elevated) return problem(403, 'elevation_required', { params: { method } })
      return new HttpResponse(null, { status: 204 })
    }),
    http.post(`${API}/me/elevate`, async ({ request }) => {
      elevations.push(await request.json())
      elevated = true
      return HttpResponse.json({ expires_in: 300 })
    }),
  )
  return { calls, elevations }
}

async function startReset() {
  const { user } = await renderApp(`/users/${ADMIN_ID}`)
  await user.click(await screen.findByRole('button', { name: 'Réinitialiser le mot de passe' }))
  const dialog = await screen.findByRole('dialog')
  await user.type(within(dialog).getByLabelText('Mot de passe'), 'another long password')
  await user.click(within(dialog).getByRole('button', { name: 'Réinitialiser le mot de passe' }))
  return { user, dialog, confirm: await screen.findByRole('dialog', { name: 'Confirmez votre identité' }) }
}

describe('session elevation', () => {
  it('asks for the password, then replays the protected action', async () => {
    const { calls, elevations } = protectedReset('password')
    const { user, dialog, confirm } = await startReset()

    // Au-dessus de la fenêtre qui a lancé l'action, restée ouverte : à `z-index` égal, la
    // dernière dans le document l'emporte.
    expect(dialog.isConnected).toBe(true)
    expect(dialog.compareDocumentPosition(confirm) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(within(confirm).getByText(/Saisissez votre mot de passe/)).toBeTruthy()
    await user.type(within(confirm).getByLabelText('Mot de passe'), 'correct horse battery')
    await user.click(within(confirm).getByRole('button', { name: 'Confirmer' }))

    expect(await screen.findByText('Mot de passe de Alice réinitialisé.')).toBeTruthy()
    expect(elevations).toEqual([{ password: 'correct horse battery' }])
    expect(calls).toHaveLength(2)
    expect(calls[1]).toEqual({ password: 'another long password' })
  })

  it('asks for the TOTP code and confirms on the sixth digit', async () => {
    const { calls, elevations } = protectedReset('totp')
    const { user, confirm } = await startReset()

    const input = within(confirm).getByLabelText('Code')
    await waitFor(() => expect(document.activeElement).toBe(input))
    await user.keyboard('123456')

    expect(await screen.findByText('Mot de passe de Alice réinitialisé.')).toBeTruthy()
    expect(elevations).toEqual([{ code: '123456' }])
    expect(calls).toHaveLength(2)
  })

  it('accepts a recovery code instead of the TOTP code', async () => {
    const { elevations } = protectedReset('totp')
    const { user, confirm } = await startReset()

    await user.click(within(confirm).getByRole('button', { name: 'Utiliser un code de secours' }))
    await user.type(within(confirm).getByLabelText('Code de secours'), 'abcd-efgh-ijkl')
    await user.click(within(confirm).getByRole('button', { name: 'Confirmer' }))

    expect(await screen.findByText('Mot de passe de Alice réinitialisé.')).toBeTruthy()
    expect(elevations).toEqual([{ code: 'abcd-efgh-ijkl' }])
  })

  it('shows a refused password under its field and lets the admin retry', async () => {
    let attempts = 0
    const { calls } = protectedReset('password')
    server.use(
      http.post(`${API}/me/elevate`, () => {
        attempts += 1
        return problem(422, 'invalid_credentials', {
          fields: [{ field: 'password', code: 'invalid_credentials', params: {} }],
        })
      }),
    )
    const { user, confirm } = await startReset()

    await user.type(within(confirm).getByLabelText('Mot de passe'), 'wrong password')
    await user.click(within(confirm).getByRole('button', { name: 'Confirmer' }))

    expect(await within(confirm).findByText('Identifiants invalides.')).toBeTruthy()
    expect(attempts).toBe(1)
    expect(calls).toHaveLength(1)
  })

  it('drops the action silently when the confirmation is cancelled', async () => {
    const { calls, elevations } = protectedReset('password')
    const { user, confirm } = await startReset()

    await user.click(within(confirm).getByRole('button', { name: 'Annuler' }))

    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Confirmez votre identité' })).toBeNull())
    expect(screen.queryByText('Confirmez votre identité pour effectuer cette action.')).toBeNull()
    expect(screen.queryByText('Mot de passe de Alice réinitialisé.')).toBeNull()
    expect(elevations).toEqual([])
    expect(calls).toHaveLength(1)
  })
})
