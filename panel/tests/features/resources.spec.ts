// Ressources, quota disque et suspension d'un serveur (ADR-0019).
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const settings = {
  default_memory_mb: 2048,
  max_memory_mb: 8192,
  default_cpu_percent: 200,
  max_cpu_percent: null,
  default_disk_mb: 10240,
  max_disk_mb: null,
  default_backup_limit: 3,
  max_backup_limit: null,
  audit_retention_days: 365,
  updated_at: '2026-10-27T10:00:00Z',
}

function setup(overrides = {}) {
  let current = serverDto(overrides)
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(current)),
    http.get(`${API}/settings`, () => HttpResponse.json(settings)),
  )
  return { set: (next: ReturnType<typeof serverDto>) => (current = next) }
}

describe('server resources', () => {
  it('sends only the changed resources and shows the project ceiling', async () => {
    setup()
    let body: unknown
    server.use(
      http.patch(`${API}/servers/${SERVER_ID}/resources`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json(serverDto({ memory_mb: 4096 }))
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}/resources`)

    expect(await screen.findByText('Plafond du projet : 8192')).toBeTruthy()
    const memory = screen.getByLabelText('Mémoire (Mo)')
    await user.clear(memory)
    await user.type(memory, '4096')
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ memory_mb: 4096 }))
    expect(await screen.findByText('Ressources enregistrées.')).toBeTruthy()
  })

  it('shows the disk usage against the quota', async () => {
    setup({ disk_mb: 10240, disk_used_bytes: 2 * 1024 ** 3 })
    await renderApp(`/servers/${SERVER_ID}/resources`)

    const meter = await screen.findByRole('meter', { name: 'Espace disque utilisé' })
    expect(meter.getAttribute('aria-valuenow')).toBe('20')
    expect(screen.getByText(/2 Go sur 10 Go/)).toBeTruthy()
  })

  it('warns on the overview when the quota is reached', async () => {
    setup({ disk_mb: 1024, disk_used_bytes: 1024 ** 3 })
    await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByText(/Quota disque atteint/)).toBeTruthy()
  })

  it('suspends after confirmation, then blocks the start', async () => {
    const state = setup()
    let suspended = false
    server.use(
      http.post(`${API}/servers/${SERVER_ID}/suspend`, () => {
        suspended = true
        const next = serverDto({ suspended_at: '2026-10-27T12:00:00Z' })
        state.set(next)
        return HttpResponse.json(next)
      }),
    )
    const { user, router } = await renderApp(`/servers/${SERVER_ID}/resources`)

    await user.click(await screen.findByRole('button', { name: 'Suspendre' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(suspended).toBe(false)
    await user.click(within(dialog).getByRole('button', { name: 'Suspendre' }))

    await waitFor(() => expect(suspended).toBe(true))
    expect(await screen.findByRole('button', { name: 'Réactiver' })).toBeTruthy()
    await router.push(`/servers/${SERVER_ID}`)
    expect(await screen.findByText(/Serveur suspendu : il ne peut ni démarrer/)).toBeTruthy()
    expect(screen.getAllByText('Suspendu').length).toBeGreaterThan(0)
    expect(screen.getByRole('button', { name: 'Démarrer' }).hasAttribute('disabled')).toBe(true)
  })

  it('is read only without the update permission and hides the suspension', async () => {
    setup()
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({
          user: { id: 'x', email: 'r@example.com', display_name: 'R', totp_enabled: false, disabled: false, created_at: '2026-10-01T00:00:00Z', roles: [] },
          permissions: ['servers.read'],
          all_permissions: false,
        }),
      ),
    )
    await renderApp(`/servers/${SERVER_ID}/resources`)

    const memory = await screen.findByLabelText('Mémoire (Mo)')
    expect((memory as HTMLInputElement).disabled).toBe(true)
    expect(screen.queryByRole('button', { name: 'Enregistrer' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Suspendre' })).toBeNull()
  })
})
