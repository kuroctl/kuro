// Historique des stats d'un serveur (ADR-0043) : courbes sur 24 h, changement de plage, et
// message quand le nœud n'a encore rien envoyé.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const PAGE = `/servers/${SERVER_ID}/stats`

function point(at: string, cpu: number, memory: number): Schemas['MetricPointDto'] {
  return {
    at,
    samples: 6,
    cpu_millis_avg: cpu,
    cpu_millis_max: cpu * 2,
    memory_bytes_avg: memory,
    memory_bytes_max: memory + 1024,
    memory_limit_bytes: 2048 * 1024 * 1024,
    network_rx_bytes: 1000,
    network_tx_bytes: 500,
  }
}

function setup(points = [point('2026-10-30T10:00:00Z', 250, 1024 * 1024 * 512), point('2026-10-30T10:01:00Z', 500, 1024 * 1024 * 700)]) {
  const ranges: string[] = []
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
    http.get(`${API}/servers/${SERVER_ID}/content`, () => HttpResponse.json([])),
    http.get(`${API}/servers/${SERVER_ID}/metrics`, ({ request }) => {
      const range = new URL(request.url).searchParams.get('range') ?? '24h'
      ranges.push(range)
      return HttpResponse.json({ range, step_secs: range === '24h' ? 60 : 3600, points: range === '24h' ? points : [] })
    }),
  )
  return ranges
}

describe('server stats history', () => {
  it('draws the day at the minute and switches to the week at the hour', async () => {
    const ranges = setup()
    const { user } = await renderApp(PAGE)

    expect((await screen.findAllByText('Moyenne')).length).toBe(2)
    expect(screen.getByText('Processeur (alloué : 2 cœurs)')).toBeTruthy()
    // Dernier point en légende : 50 % de CPU, 700 Mio de mémoire.
    expect(screen.getAllByText('50 %').length).toBeGreaterThan(0)
    expect(screen.getAllByText(/700/).length).toBeGreaterThan(0)
    await waitFor(() => {
      const nav = screen.queryAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))
      expect(within(nav!).getByRole('link', { name: 'Statistiques' })).toBeTruthy()
    })

    await user.click(screen.getByRole('combobox', { name: 'Plage' }))
    await user.click(await screen.findByRole('option', { name: '7 jours' }))
    await waitFor(() => expect(ranges).toContain('7d'))
    expect(await screen.findByText('Pas encore d’historique : le nœud envoie une minute à la fois, dès qu’un serveur tourne.')).toBeTruthy()
  })
})
