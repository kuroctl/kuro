// Envoi d'e-mails (ADR-0046) : réglages SMTP enregistrés sans renvoyer le mot de passe, e-mail
// de test, retrait ; canal e-mail des notifications avec ses destinataires.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, mailNotConfigured, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

const configured: Schemas['MailSettingsDto'] = {
  configured: true,
  insecure_allowed: false,
  host: 'smtp.example.com',
  port: 587,
  security: 'starttls',
  username: 'kuro',
  has_password: true,
  from_address: 'kuro@example.com',
  from_name: null,
  updated_at: '2026-10-31T10:00:00Z',
}

describe('e-mail settings', () => {
  it('saves the SMTP server without ever showing the password, then tests it', async () => {
    let body: unknown
    let tested = false
    server.use(
      ...signedIn(),
      http.get(`${API}/settings/mail`, () => HttpResponse.json(mailNotConfigured)),
      http.put(`${API}/settings/mail`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(configured)
      }),
      http.post(`${API}/settings/mail/test`, () => {
        tested = true
        return HttpResponse.json({ ok: true, to: 'admin@example.com' })
      }),
    )
    const { user } = await renderApp('/settings/mail')

    expect(await screen.findByText('Non configuré')).toBeTruthy()
    await user.type(screen.getByLabelText('Serveur'), 'smtp.example.com')
    await user.type(screen.getByLabelText('Identifiant (facultatif)'), 'kuro')
    await user.type(screen.getByLabelText('Mot de passe (facultatif)'), 's3cret')
    await user.type(screen.getByLabelText('Adresse d’expédition'), 'kuro@example.com')
    await user.click(screen.getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({
        host: 'smtp.example.com',
        port: 587,
        security: 'starttls',
        username: 'kuro',
        password: 's3cret',
        from_address: 'kuro@example.com',
        from_name: null,
      }),
    )
    expect(await screen.findByText('Serveur SMTP enregistré.')).toBeTruthy()
    // Une fois en place : le champ du mot de passe est vide, le test est proposé.
    server.use(http.get(`${API}/settings/mail`, () => HttpResponse.json(configured)))
    expect(await screen.findByText('Configuré')).toBeTruthy()
    expect((screen.getByLabelText('Nouveau mot de passe') as HTMLInputElement).value).toBe('')
    await user.click(screen.getByRole('button', { name: 'Envoyer un e-mail de test' }))
    await waitFor(() => expect(tested).toBe(true))
    expect(await screen.findByText('E-mail de test envoyé à admin@example.com.')).toBeTruthy()
  })

  it('shows the SMTP refusal of a test as a toast with its cause', async () => {
    // Le réglage par défaut de `signedIn()` dit « non configuré » : le nôtre passe devant.
    server.use(
      http.get(`${API}/settings/mail`, () => HttpResponse.json(configured)),
      http.post(`${API}/settings/mail/test`, () => problem(502, 'mail_send_failed', { params: { error: 'smtp: 535 authentication failed' } })),
      ...signedIn(),
    )
    const { user } = await renderApp('/settings/mail')

    await user.click(await screen.findByRole('button', { name: 'Envoyer un e-mail de test' }))
    expect(await screen.findByText('Le serveur SMTP a refusé l’envoi : smtp: 535 authentication failed')).toBeTruthy()
  })

  it('creates an e-mail notification channel from one address per line', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings/notifications/channels`, () => HttpResponse.json([])),
      http.get(`${API}/settings/notifications/rules`, () => HttpResponse.json([])),
      http.get(`${API}/settings/notifications/events`, () => HttpResponse.json(['server.crashed'])),
      http.get(`${API}/settings/notifications/deliveries`, () => HttpResponse.json([])),
      http.get(`${API}/nodes`, () => HttpResponse.json([])),
      http.get(`${API}/servers`, () => HttpResponse.json([])),
      http.post(`${API}/settings/notifications/channels`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(
          {
            id: '01a17f00-0000-7000-8000-0000000000e1',
            name: 'Ops',
            kind: 'email',
            url_hint: 'o…@example.com (+1)',
            has_secret: false,
            language: 'fr',
            enabled: true,
            created_at: '2026-10-31T10:00:00Z',
            updated_at: '2026-10-31T10:00:00Z',
          },
          { status: 201 },
        )
      }),
    )
    const { user } = await renderApp('/settings/notifications')

    await user.click(await screen.findByRole('button', { name: 'Ajouter un canal' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Ops')
    await user.click(within(dialog).getByRole('combobox', { name: 'Type' }))
    await user.click(await screen.findByRole('option', { name: 'E-mail' }))
    await user.type(within(dialog).getByLabelText('Destinataires'), 'ops@example.com\nboss@example.com')
    await user.click(within(dialog).getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toMatchObject({ kind: 'email', recipients: ['ops@example.com', 'boss@example.com'] }))
    expect(body).not.toHaveProperty('url')
  })
})
