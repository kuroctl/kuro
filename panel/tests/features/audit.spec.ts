// Journal d'audit (ADR-0019) : liste filtrée et paginée, détail, vérification, export CSV.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

function entry(seq: number, overrides: Partial<Schemas['AuditEntryDto']> = {}): Schemas['AuditEntryDto'] {
  return {
    seq,
    id: `01a17da3-7b31-744d-850c-${String(seq).padStart(12, '0')}`,
    occurred_at: '2026-10-27T10:00:00Z',
    actor_kind: 'admin',
    actor_id: profile.user.id,
    action: 'server.power',
    outcome: 'success',
    target_kind: 'server',
    target_id: '01a17a00-0000-7000-8000-0000000000aa',
    ip: '203.0.113.7',
    user_agent: 'Firefox',
    request_id: 'abc',
    details: { action: 'start' },
    hash: 'f'.repeat(64),
    ...overrides,
  }
}

describe('audit log', () => {
  it('lists the events and filters them by outcome', async () => {
    const queries: URLSearchParams[] = []
    server.use(
      ...signedIn(),
      http.get(`${API}/audit`, ({ request }) => {
        const query = new URL(request.url).searchParams
        queries.push(query)
        const items = query.get('outcome') === 'denied' ? [entry(2, { action: 'access.denied', outcome: 'denied' })] : [entry(1)]
        return HttpResponse.json({ items, next_before_seq: null })
      }),
    )
    const { user } = await renderApp('/settings/audit')

    expect(await screen.findByText('server.power')).toBeTruthy()
    expect(screen.getByText('Vous')).toBeTruthy()
    await user.click(screen.getByRole('combobox', { name: 'Issue' }))
    await user.click(await screen.findByRole('option', { name: 'Refusé' }))

    expect(await screen.findByText('access.denied')).toBeTruthy()
    expect(queries.at(-1)?.get('outcome')).toBe('denied')
  })

  it('keeps the filters in the URL and restores them on load', async () => {
    const queries: URLSearchParams[] = []
    server.use(
      ...signedIn(),
      http.get(`${API}/audit`, ({ request }) => {
        queries.push(new URL(request.url).searchParams)
        return HttpResponse.json({ items: [entry(2, { action: 'access.denied', outcome: 'denied' })], next_before_seq: null })
      }),
    )
    const { user, router } = await renderApp('/settings/audit?outcome=denied&actor_kind=system')

    expect(await screen.findByText('access.denied')).toBeTruthy()
    expect(queries[0]?.get('outcome')).toBe('denied')
    expect(queries[0]?.get('actor_kind')).toBe('system')
    expect(screen.getByRole('combobox', { name: 'Issue' }).textContent).toContain('Refusé')

    await user.click(screen.getByRole('button', { name: 'Réinitialiser' }))
    await waitFor(() => expect(router.currentRoute.value.query).toEqual({}))
    await waitFor(() => expect(queries.at(-1)?.get('outcome')).toBeNull())
  })

  it('shows the columns in the shared table, without sorting, and hides or shows them', async () => {
    window.localStorage.removeItem('kuro.table.audit')
    server.use(...signedIn(), http.get(`${API}/audit`, () => HttpResponse.json({ items: [entry(1)], next_before_seq: null })))
    const { user } = await renderApp('/settings/audit')

    await screen.findByText('server.power')
    const headers = () => screen.getAllByRole('columnheader').map((cell) => cell.textContent?.trim())
    expect(headers()).toEqual(['Date', 'Acteur', 'Action', 'Issue', 'Cible'])
    expect(within(screen.getByRole('table')).queryAllByRole('button')).toHaveLength(0)

    await user.click(screen.getByRole('button', { name: 'Colonnes' }))
    await user.click(await screen.findByRole('menuitemcheckbox', { name: 'Adresse IP' }))
    await user.click(screen.getByRole('menuitemcheckbox', { name: 'Cible' }))
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull())
    await waitFor(() => expect(headers()).toEqual(['Date', 'Acteur', 'Action', 'Issue', 'Adresse IP']))
    expect(screen.getByText('203.0.113.7')).toBeTruthy()
    expect(JSON.parse(window.localStorage.getItem('kuro.table.audit') ?? '{}')).toMatchObject({ ip: true, target: false })
    window.localStorage.removeItem('kuro.table.audit')
  })

  it('loads older events on demand', async () => {
    server.use(
      ...signedIn(),
      http.get(`${API}/audit`, ({ request }) => {
        const before = new URL(request.url).searchParams.get('before_seq')
        return before === '10'
          ? HttpResponse.json({ items: [entry(9, { action: 'auth.logout' })], next_before_seq: null })
          : HttpResponse.json({ items: [entry(10)], next_before_seq: 10 })
      }),
    )
    const { user } = await renderApp('/settings/audit')

    await user.click(await screen.findByRole('button', { name: 'Événements plus anciens' }))

    expect(await screen.findByText('auth.logout')).toBeTruthy()
    expect(screen.getByText('server.power')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Événements plus anciens' })).toBeNull()
  })

  it('opens the full context of an event', async () => {
    server.use(...signedIn(), http.get(`${API}/audit`, () => HttpResponse.json({ items: [entry(1)], next_before_seq: null })))
    const { user, router } = await renderApp('/settings/audit?outcome=success')

    await user.click(await screen.findByText('server.power'))

    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByText('203.0.113.7')).toBeTruthy()
    expect(within(dialog).getByText('Firefox')).toBeTruthy()
    expect(within(dialog).getByText(/"action": "start"/)).toBeTruthy()
    expect(router.currentRoute.value.query).toEqual({ outcome: 'success', entry: '1' })

    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
    expect(router.currentRoute.value.query).toEqual({ outcome: 'success' })
  })

  it('verifies the hash chain', async () => {
    server.use(
      ...signedIn(),
      http.get(`${API}/audit`, () => HttpResponse.json({ items: [entry(1)], next_before_seq: null })),
      http.post(`${API}/audit/verify`, () =>
        HttpResponse.json({ intact: false, checked: 42, first_seq: 1, last_seq: 42, broken_at: 17 }),
      ),
    )
    const { user } = await renderApp('/settings/audit')

    await user.click(await screen.findByRole('button', { name: 'Vérifier l’intégrité' }))

    expect(await screen.findByText('Chaîne rompue à l’événement n° 17 : le journal a été modifié.')).toBeTruthy()
  })

  it('exports the filtered events as a CSV file', async () => {
    let exported: URLSearchParams | undefined
    server.use(
      ...signedIn(),
      http.get(`${API}/audit`, () => HttpResponse.json({ items: [entry(1)], next_before_seq: null })),
      http.get(`${API}/audit/export`, ({ request }) => {
        exported = new URL(request.url).searchParams
        return new HttpResponse('seq,occurred_at\r\n1,2026-10-27T10:00:00Z\r\n', {
          headers: { 'content-type': 'text/csv; charset=utf-8', 'content-disposition': 'attachment; filename="kuro-audit-2026-10-27.csv"' },
        })
      }),
    )
    const createObjectURL = vi.fn(() => 'blob:audit')
    const revokeObjectURL = vi.fn()
    Object.assign(URL, { createObjectURL, revokeObjectURL })
    const clicked: string[] = []
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      clicked.push(this.download)
    })
    const { user } = await renderApp('/settings/audit')

    await user.click(await screen.findByRole('combobox', { name: 'Acteur' }))
    await user.click(await screen.findByRole('option', { name: 'Système' }))
    await user.click(screen.getByRole('button', { name: 'Exporter en CSV' }))

    await waitFor(() => expect(clicked).toEqual(['kuro-audit-2026-10-27.csv']))
    expect(exported?.get('actor_kind')).toBe('system')
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:audit')
    click.mockRestore()
  })

  it('tells an admin without access', async () => {
    server.use(...signedIn())
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read'] })),
    )
    await renderApp('/settings/audit')

    expect(await screen.findByText('Vous n’avez pas accès au journal d’audit.')).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Exporter en CSV' })).toBeNull()
  })
})
