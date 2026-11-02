// Extensions du panel admin (ADR-0051 § 8) : l'extension de démonstration `demo`
// (`tests/fixtures/panel-extension`) ajoute une section au rail, une entrée aux Utilisateurs et
// des pages sous `/demo/`, avec ses traductions ; sa section suit sa permission, et son sous-menu
// range une page dans un groupe.
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

  it('shows the groups of its menu, each page with its permission', async () => {
    server.use(...signedIn())
    server.use(withPermissions(['demo.notes.read']))
    const { user, router } = await renderApp('/demo/notes')

    expect(await screen.findByRole('button', { name: 'Classement' })).toBeTruthy()
    expect(screen.queryByRole('link', { name: 'Secret' })).toBeNull()

    await user.click(screen.getByRole('link', { name: 'Archives' }))

    expect(await screen.findByRole('heading', { name: 'Archives de la démo' })).toBeTruthy()
    expect(router.currentRoute.value.path).toBe('/demo/notes/archive')
    expect(screen.getByRole('link', { name: 'Archives' }).getAttribute('data-active')).toBe('true')
    expect(screen.getByRole('link', { name: 'Notes' }).getAttribute('data-active')).not.toBe('true')
  })

  it('opens the folded group of the page reached', async () => {
    server.use(...signedIn())
    const { user, router } = await renderApp('/demo/notes')

    await user.click(await screen.findByRole('button', { name: 'Classement' }))
    expect(screen.queryByRole('link', { name: 'Archives' })).toBeNull()

    await router.push('/demo/notes/archive')

    expect(await screen.findByRole('link', { name: 'Archives' })).toBeTruthy()
  })

  it('finds the pages of a group with the menu search', async () => {
    server.use(...signedIn())
    const { user } = await renderApp('/demo/notes')

    await user.click(await screen.findByRole('button', { name: 'Classement' }))
    await user.type(screen.getByRole('searchbox', { name: 'Rechercher' }), 'archi')

    expect(await screen.findByRole('link', { name: 'Archives' })).toBeTruthy()
    expect(screen.queryByRole('link', { name: 'Notes' })).toBeNull()
  })

  it('speaks the language of the panel', async () => {
    applyLocale('en')
    server.use(...signedIn())
    await renderApp('/demo/notes')

    expect(await screen.findByRole('heading', { name: 'Demo notes' })).toBeTruthy()
    expect(screen.getByRole('link', { name: 'Demo' })).toBeTruthy()
  })
})
