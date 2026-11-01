// Paramètres du projet (ADR-0019) : valeurs par défaut, plafonds, conservation du journal.
import { screen, waitFor } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

const settings: Schemas['SettingsDto'] = {
  default_memory_mb: 2048,
  max_memory_mb: null,
  default_cpu_percent: 200,
  max_cpu_percent: null,
  default_disk_mb: 10240,
  max_disk_mb: 20480,
  default_backup_limit: 3,
  max_backup_limit: null,
  audit_retention_days: 365,
  alert_cpu_percent: 90,
  alert_memory_percent: 90,
  alert_minutes: 5,
  mcp_enabled: false,
  sftp_enabled: false,
  file_history_days: 30,
  file_history_max_mb: 100,
  mcp_url: 'https://kuro.test/mcp',
  updated_at: '2026-10-27T10:00:00Z',
}

describe('project settings', () => {
  it('sends only the changed fields, an emptied ceiling as null', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...settings, default_memory_mb: 4096, max_disk_mb: null })
      }),
    )
    const { user } = await renderApp('/settings')

    const memory = await screen.findByLabelText('Par défaut', { selector: '#default_memory_mb' })
    // Formaté selon la langue : espace fine insécable entre les milliers.
    expect((memory as HTMLInputElement).value).toBe('2 048')
    await user.clear(memory)
    await user.type(memory, '4096')
    await user.clear(screen.getByLabelText('Plafond', { selector: '#max_disk_mb' }))
    // Un NumberField valide sa saisie en perdant le focus : la barre d'enregistrement apparaît alors.
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ default_memory_mb: 4096, max_disk_mb: null }))
    expect(await screen.findByText('Paramètres enregistrés.')).toBeTruthy()
  })

  it('sends a changed alert threshold', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...settings, alert_minutes: 10 })
      }),
    )
    const { user } = await renderApp('/settings')

    expect(await screen.findByText('Seuils d’alerte')).toBeTruthy()
    expect((screen.getByLabelText('Processeur (% de l’allocation)') as HTMLInputElement).value).toBe('90')
    const minutes = screen.getByLabelText('Durée (minutes)')
    await user.clear(minutes)
    await user.type(minutes, '10')
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ alert_minutes: 10 }))
  })

  it('edits the CPU in cores and sends it as a percentage', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...settings, default_cpu_percent: 50, max_cpu_percent: 800 })
      }),
    )
    const { user } = await renderApp('/settings')

    expect(await screen.findByText('En cœurs, décimales permises (0,5 = un demi-cœur).')).toBeTruthy()
    const cpu = screen.getByLabelText('Par défaut', { selector: '#default_cpu_percent' }) as HTMLInputElement
    expect(cpu.value).toBe('2')
    await user.clear(cpu)
    await user.type(cpu, '0,5')
    await user.type(screen.getByLabelText('Plafond', { selector: '#max_cpu_percent' }), '8')
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ default_cpu_percent: 50, max_cpu_percent: 800 }))
    expect(cpu.value).toBe('0,5')
  })

  it('shows the CPU bounds refused by the API in cores', async () => {
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, () =>
        problem(422, 'validation_failed', {
          fields: [{ field: 'default_cpu_percent', code: 'value_out_of_range', params: { min: 1, max: 12_800 } }],
        }),
      ),
    )
    const { user } = await renderApp('/settings')

    const cpu = await screen.findByLabelText('Par défaut', { selector: '#default_cpu_percent' })
    await user.clear(cpu)
    await user.type(cpu, '3')
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    expect(await screen.findByText('Valeur hors bornes : entre 0,01 cœur et 128 cœurs.')).toBeTruthy()
  })

  it('shows the refusal of a field under it', async () => {
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, () =>
        problem(422, 'validation_failed', {
          fields: [{ field: 'audit_retention_days', code: 'value_out_of_range', params: { min: 30, max: 3650 } }],
        }),
      ),
    )
    const { user } = await renderApp('/settings')

    const retention = await screen.findByLabelText('Conservation (jours)')
    await user.clear(retention)
    await user.type(retention, '7')
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    expect(await screen.findByText('Valeur hors bornes : entre 30 et 3650.')).toBeTruthy()
  })

  it('is read only without the update permission', async () => {
    server.use(...signedIn(), http.get(`${API}/settings`, () => HttpResponse.json(settings)))
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['system.settings.read'] }),
      ),
    )
    await renderApp('/settings')

    const memory = await screen.findByLabelText('Par défaut', { selector: '#default_memory_mb' })
    expect((memory as HTMLInputElement).disabled).toBe(true)
    expect(screen.queryByRole('button', { name: 'Enregistrer' })).toBeNull()
  })

  it('tells an admin without access instead of loading', async () => {
    server.use(...signedIn())
    server.use(
      http.get(`${API}/me`, () => HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read'] })),
    )
    await renderApp('/settings')

    expect(await screen.findByText('Vous n’avez pas accès aux paramètres du projet.')).toBeTruthy()
  })

  it('sends a new file history retention', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...settings, file_history_days: 7 })
      }),
    )
    const { user } = await renderApp('/settings')

    expect(await screen.findByText('Historique des fichiers')).toBeTruthy()
    const days = screen.getByLabelText('Conservation des versions (jours)')
    await user.clear(days)
    await user.type(days, '7')
    await user.tab()
    await user.click(await screen.findByRole('button', { name: 'Enregistrer' }))

    await waitFor(() => expect(body).toEqual({ file_history_days: 7 }))
  })
})

