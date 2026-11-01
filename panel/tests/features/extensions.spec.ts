// Extensions du panel admin (ADR-0051 § 8) : l'extension de démonstration `demo`
// (`tests/fixtures/panel-extension`) ajoute une section au rail, une entrée aux Utilisateurs et
// une page sous `/demo/`, avec ses traductions ; sa section suit sa permission.
import { screen } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { applyLocale } from '@/shared/i18n'

import { API, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

function withPermissions(permissions: string[]) {
  return http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions }))
}

describe('panel extensions', () => {
  it('adds a rail section and serves its pages under its name', async () => {
    server.use(...signedIn())
    const { user, router } = await renderApp('/account')

    await user.click(await screen.findByRole('link', { name: 'Démo' }))

    expect(await screen.findByRole('heading', { name: 'Notes de la démo' })).toBeTruthy()
    expect(router.currentRoute.value.path).toBe('/demo/notes')
  })

  it('adds an entry to a section of Kuro', async () => {
    server.use(...signedIn())
    await renderApp('/users')

    expect(await screen.findByRole('link', { name: 'Clients de la démo' })).toBeTruthy()
  })

  it('hides the extension without its permission', async () => {
    server.use(...signedIn())
    server.use(withPermissions(['servers.read']))
    await renderApp('/account')

    await screen.findAllByRole('link', { name: 'Serveurs' })
    expect(screen.queryByRole('link', { name: 'Démo' })).toBeNull()
  })

  it('speaks the language of the panel', async () => {
    applyLocale('en')
    server.use(...signedIn())
    await renderApp('/demo/notes')

    expect(await screen.findByRole('heading', { name: 'Demo notes' })).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Demo' })).toBeTruthy()
  })
})
