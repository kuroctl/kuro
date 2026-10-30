// Jeton d'accès expiré : un seul rafraîchissement pour les requêtes concurrentes, puis retour à la
// connexion si la session ne peut pas être renouvelée.
import { waitFor } from '@testing-library/vue'
import { delay, http as mock, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { http } from '@/shared/api/http'

import { accessToken, API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

describe('session renewal', () => {
  it('refreshes an expired token once and replays concurrent requests', async () => {
    server.use(...signedIn())
    await renderApp('/servers')

    let refreshes = 0
    server.use(
      mock.post(`${API}/auth/refresh`, () => {
        refreshes += 1
        return HttpResponse.json(accessToken('token-2'))
      }),
      mock.get(`${API}/me`, ({ request }) =>
        request.headers.get('authorization') === 'Bearer token-2'
          ? HttpResponse.json(profile)
          : problem(401, 'unauthorized'),
      ),
    )

    const responses = await Promise.all([http.get('/admin/v1/me'), http.get('/admin/v1/me')])

    expect(responses.map((response) => response.status)).toEqual([200, 200])
    expect(refreshes).toBe(1)
  })

  it('sends the admin back to the login page when the session cannot be renewed', async () => {
    server.use(...signedIn())
    const { router } = await renderApp('/servers')

    server.use(
      mock.post(`${API}/auth/refresh`, () => problem(401, 'unauthorized')),
      mock.get(`${API}/me`, () => problem(401, 'unauthorized')),
    )
    await expect(http.get('/admin/v1/me')).rejects.toThrow()

    await waitFor(() => expect(router.currentRoute.value.path).toBe('/login'))
    expect(router.currentRoute.value.query.redirect).toBe('/servers')
  })

  it('never tries to refresh on an authentication route', async () => {
    let refreshes = 0
    server.use(
      mock.post(`${API}/auth/refresh`, () => {
        refreshes += 1
        return problem(401, 'unauthorized')
      }),
      mock.post(`${API}/auth/login`, () => problem(401, 'invalid_credentials')),
    )
    await renderApp('/login')

    await expect(http.post('/admin/v1/auth/login', {})).rejects.toThrow()
    expect(refreshes).toBe(0)
  })
})

describe('session shared between tabs', () => {
  it('lets two tabs resume the session at the same time without tripping the reuse detection', async () => {
    // L'API fait tourner le cookie : un rafraîchissement reçu pendant un autre réutilise le même
    // cookie, et l'API révoque alors toute la session.
    let inFlight = 0
    let reused = false
    server.use(
      mock.post(`${API}/auth/refresh`, async () => {
        inFlight += 1
        if (inFlight > 1) reused = true
        await delay(20)
        inFlight -= 1
        return reused ? problem(401, 'unauthorized') : HttpResponse.json(accessToken())
      }),
      mock.get(`${API}/me`, () => HttpResponse.json(profile)),
    )

    const tabs = await Promise.all([renderApp('/account'), renderApp('/account')])

    expect(reused).toBe(false)
    expect(tabs.map(({ router }) => router.currentRoute.value.path)).toEqual(['/account', '/account'])
  })
})
