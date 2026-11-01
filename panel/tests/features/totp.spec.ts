// Vérification en deux étapes gérée depuis le compte (ADR-0007) : activation, changement
// d'appareil, codes de secours, désactivation.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

// Le QR code passe par un canvas, absent de jsdom.
vi.mock('qrcode', () => ({ default: { toDataURL: async () => 'data:image/png;base64,AAAA' } }))

const CODES = ['abcd-efgh-ijkl', 'mnop-qrst-uvwx']

function withTotp(extra: Partial<Schemas['ProfileDto']> = {}): Schemas['ProfileDto'] {
  return { ...profile, user: { ...profile.user, totp_enabled: true }, recovery_codes_remaining: 8, ...extra }
}

describe('two-step verification from the account', () => {
  it('sets up TOTP for an account without one and shows the recovery codes', async () => {
    let me = profile
    let started: unknown
    let confirmed: unknown
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json(me)),
      http.post(`${API}/me/totp/setup`, async ({ request }) => {
        started = await request.json()
        return HttpResponse.json({ secret: 'JBSWY3DPEHPK3PXP', otpauth_uri: 'otpauth://totp/Kuro:admin' })
      }),
      http.post(`${API}/me/totp/confirm`, async ({ request }) => {
        confirmed = await request.json()
        me = withTotp({ recovery_codes_remaining: 10 })
        return HttpResponse.json({ recovery_codes: CODES })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    expect(await screen.findByText('Non configurée')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Configurer' }))
    const dialog = await screen.findByRole('dialog')
    // Aucun TOTP encore : le mot de passe suffit.
    expect(within(dialog).queryByLabelText('Code actuel ou code de secours')).toBeNull()
    await user.type(within(dialog).getByLabelText('Mot de passe'), 'correct horse battery')
    await user.click(within(dialog).getByRole('button', { name: 'Continuer' }))

    expect(await within(dialog).findByText('JBSWY3DPEHPK3PXP')).toBeTruthy()
    expect(started).toEqual({ password: 'correct horse battery', code: null })
    // Le champ prend le focus seul, et le sixième chiffre valide sans passer par le bouton.
    const input = within(dialog).getByLabelText('Code affiché')
    await waitFor(() => expect(document.activeElement).toBe(input))
    await user.keyboard('654321')

    expect(await within(dialog).findByText('abcd-efgh-ijkl')).toBeTruthy()
    expect(confirmed).toEqual({ code: '654321' })
    expect(await screen.findByText('Vérification en deux étapes activée.')).toBeTruthy()
    await user.click(within(dialog).getByRole('checkbox'))
    await user.click(within(dialog).getByRole('button', { name: 'Terminer' }))

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(await screen.findByText('10 codes de secours restants')).toBeTruthy()
    expect(screen.getByText('Activée')).toBeTruthy()
  })

  it('asks for the current code to change device and shows a refused one under its field', async () => {
    let attempts = 0
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json(withTotp())),
      http.post(`${API}/me/totp/setup`, async ({ request }) => {
        attempts += 1
        const body = (await request.json()) as Schemas['TotpReauthRequest']
        if (body.code !== '123456') {
          return problem(422, 'invalid_code', { fields: [{ field: 'code', code: 'invalid_code', params: {} }] })
        }
        return HttpResponse.json({ secret: 'JBSWY3DPEHPK3PXP', otpauth_uri: 'otpauth://totp/Kuro:admin' })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    await user.click(await screen.findByRole('button', { name: 'Changer d’appareil' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Mot de passe'), 'correct horse battery')
    await user.type(within(dialog).getByLabelText('Code actuel ou code de secours'), '000000')
    await user.click(within(dialog).getByRole('button', { name: 'Continuer' }))

    expect(await within(dialog).findByText('Code invalide.')).toBeTruthy()
    await user.type(within(dialog).getByLabelText('Code actuel ou code de secours'), '123456')
    await user.click(within(dialog).getByRole('button', { name: 'Continuer' }))
    expect(await within(dialog).findByText('JBSWY3DPEHPK3PXP')).toBeTruthy()
    expect(attempts).toBe(2)
  })

  it('regenerates the recovery codes and warns when few are left', async () => {
    let body: unknown
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json(withTotp({ recovery_codes_remaining: 2 }))),
      http.post(`${API}/me/totp/recovery-codes`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ recovery_codes: CODES })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    expect(await screen.findByText(/Il vous reste peu de codes de secours/)).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Nouveaux codes de secours' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Mot de passe'), 'correct horse battery')
    await user.type(within(dialog).getByLabelText('Code actuel ou code de secours'), '123456')
    await user.click(within(dialog).getByRole('button', { name: 'Continuer' }))

    expect(await within(dialog).findByText('mnop-qrst-uvwx')).toBeTruthy()
    expect(body).toEqual({ password: 'correct horse battery', code: '123456' })
  })

  it('disables TOTP when the platform does not require it', async () => {
    let me = withTotp()
    let body: unknown
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json(me)),
      http.delete(`${API}/me/totp`, async ({ request }) => {
        body = await request.json()
        me = profile
        return new HttpResponse(null, { status: 204 })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    await user.click(await screen.findByRole('button', { name: 'Désactiver' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('Désactiver la vérification en deux étapes ?')).toBeTruthy()
    await user.type(within(dialog).getByLabelText('Mot de passe'), 'correct horse battery')
    await user.type(within(dialog).getByLabelText('Code actuel ou code de secours'), '123456')
    await user.click(within(dialog).getByRole('button', { name: 'Désactiver' }))

    expect(await screen.findByText('Vérification en deux étapes désactivée.')).toBeTruthy()
    expect(body).toEqual({ password: 'correct horse battery', code: '123456' })
    expect(await screen.findByRole('button', { name: 'Configurer' })).toBeTruthy()
  })

  it('offers no way to disable TOTP when the platform requires it', async () => {
    server.use(http.get(`${API}/me`, () => HttpResponse.json(withTotp({ totp_required: true }))), ...signedIn())
    await renderApp('/account/security')

    expect(await screen.findByText(/Obligatoire sur cette plateforme/)).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Changer d’appareil' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Désactiver' })).toBeNull()
  })
})
