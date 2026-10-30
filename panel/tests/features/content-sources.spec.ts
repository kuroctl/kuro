// Sources de contenu (ADR-0029, ADR-0030) : clés d'API de CurseForge et du Workshop Steam
// vérifiées par l'API, jamais réaffichées, retirées après confirmation ; page réservée à sa
// permission.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

const SOURCES = `${API}/settings/content-sources`
const PAGE = '/settings/content-sources'

function sources(configured: boolean): Schemas['ContentSourceDto'][] {
  return [{ provider: 'curseforge', configured, updated_at: configured ? '2026-10-28T10:00:00Z' : null }]
}

describe('content sources', () => {
  it('saves a CurseForge key checked by the API, then clears the field', async () => {
    let body: unknown
    let configured = false
    server.use(
      ...signedIn(),
      http.get(SOURCES, () => HttpResponse.json(sources(configured))),
      http.put(`${SOURCES}/curseforge`, async ({ request }) => {
        body = await request.json()
        configured = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(PAGE)

    expect(await screen.findByText('Sans clé')).toBeTruthy()
    const field = screen.getByLabelText('Clé d’API')
    await user.type(field, '  my-api-key  ')
    await user.click(screen.getByRole('button', { name: 'Enregistrer la clé' }))

    await waitFor(() => expect(body).toEqual({ api_key: 'my-api-key' }))
    expect(await screen.findByText('Clé CurseForge vérifiée et enregistrée.')).toBeTruthy()
    expect(await screen.findByLabelText('Remplacer la clé')).toHaveProperty('value', '')
    expect(screen.getAllByText('Active')).toHaveLength(2)
  })

  it('shows a key refused by CurseForge under the field', async () => {
    server.use(
      ...signedIn(),
      http.get(SOURCES, () => HttpResponse.json(sources(false))),
      http.put(`${SOURCES}/curseforge`, () =>
        problem(422, 'validation_failed', { fields: [{ field: 'api_key', code: 'content_source_unauthorized', params: {} }] }),
      ),
    )
    const { user } = await renderApp(PAGE)

    await user.type(await screen.findByLabelText('Clé d’API'), 'wrong')
    await user.click(screen.getByRole('button', { name: 'Enregistrer la clé' }))

    expect(await screen.findByText('La source a refusé cette clé d’API : vérifiez-la dans sa console.')).toBeTruthy()
  })

  it('removes the key after confirmation', async () => {
    let removed = false
    server.use(
      ...signedIn(),
      http.get(SOURCES, () => HttpResponse.json(sources(!removed))),
      http.delete(`${SOURCES}/curseforge`, () => {
        removed = true
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('button', { name: 'Retirer la clé' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: 'Retirer la clé' }))

    await waitFor(() => expect(removed).toBe(true))
    expect(await screen.findByText('Sans clé')).toBeTruthy()
  })

  it('keeps a key per source, Steam Workshop with its own console', async () => {
    let path = ''
    server.use(
      ...signedIn(),
      http.get(SOURCES, () =>
        HttpResponse.json([...sources(true), { provider: 'steam-workshop', configured: false, updated_at: null }]),
      ),
      http.put(`${SOURCES}/:provider`, ({ params }) => {
        path = String(params.provider)
        return new HttpResponse(null, { status: 204 })
      }),
    )
    const { user } = await renderApp(PAGE)

    const steam = await screen.findByRole('region', { name: 'Steam Workshop' })
    expect(within(steam).getByText('Sans clé')).toBeTruthy()
    expect(within(steam).getByRole('link', { name: 'steamcommunity.com' }).getAttribute('href')).toBe('https://steamcommunity.com/dev/apikey')
    expect(within(screen.getByRole('region', { name: 'CurseForge' })).getByText('Active')).toBeTruthy()

    await user.type(within(steam).getByLabelText('Clé d’API'), 'steam-key')
    await user.click(within(steam).getByRole('button', { name: 'Enregistrer la clé' }))

    await waitFor(() => expect(path).toBe('steam-workshop'))
    expect(await screen.findByText('Clé Steam Workshop vérifiée et enregistrée.')).toBeTruthy()
  })

  it('is out of reach without its permission', async () => {
    server.use(...signedIn())
    // Après `signedIn()` : dans un même appel, le premier handler de `/me` gagnerait.
    server.use(http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['system.settings.read'] })))
    await renderApp(PAGE)

    expect(await screen.findByText('Vous n’avez pas accès aux sources de contenu.')).toBeTruthy()
    expect(screen.queryByRole('link', { name: 'Sources de contenu' })).toBeNull()
  })
})
