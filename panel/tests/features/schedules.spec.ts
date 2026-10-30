// Tâches planifiées (ADR-0022) : liste (déclencheur en clair, prochaine et dernière exécution),
// activation, lancement, création avec raccourcis et étapes, actions limitées aux permissions,
// historique.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const SCHEDULES = `${API}/servers/${SERVER_ID}/schedules`
const PAGE = `/servers/${SERVER_ID}/schedules`
const NIGHT = '01a17c00-0000-7000-8000-000000000001'

function schedule(overrides: Partial<Schemas['ScheduleDto']> = {}): Schemas['ScheduleDto'] {
  return {
    id: NIGHT,
    name: 'Nuit',
    cron: '0 4 * * *',
    timezone: 'Europe/Paris',
    enabled: true,
    only_when_online: false,
    next_run_at: '2026-10-29T03:00:00Z',
    running: false,
    steps: [
      { action: 'backup', value: null, delay_secs: 0, continue_on_failure: false },
      { action: 'power', value: 'restart', delay_secs: 30, continue_on_failure: false },
    ],
    last_run: {
      id: '01a17c00-0000-7000-8000-0000000000aa',
      trigger: 'schedule',
      outcome: 'failed',
      started_at: '2026-10-28T03:00:00Z',
      finished_at: '2026-10-28T03:00:05Z',
      failed_step: 0,
      reason: 'backup_failed',
    },
    created_at: '2026-10-01T00:00:00Z',
    ...overrides,
  }
}

function setup(list = [schedule()]) {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
    http.get(SCHEDULES, () => HttpResponse.json(list)),
    http.post(`${SCHEDULES}/preview`, () =>
      HttpResponse.json({ runs: ['2026-10-29T03:00:00Z', '2026-10-30T03:00:00Z', '2026-10-31T03:00:00Z'] }),
    ),
  )
}

describe('scheduled tasks', () => {
  it('lists each task with its trigger in plain words and its last run', async () => {
    setup()
    await renderApp(PAGE)

    const card = (await screen.findByText('Nuit')).closest<HTMLElement>('[data-schedule]')!
    expect(within(card).getByText('Tous les jours à 04:00')).toBeTruthy()
    expect(within(card).getByText(/Prochaine exécution/)).toBeTruthy()
    expect(within(card).getByText('Échec')).toBeTruthy()
    expect(within(card).getByText('Étape 1 : La sauvegarde a échoué')).toBeTruthy()
    expect(within(card).getByText('Redémarrer')).toBeTruthy()
    const link = await waitFor(() => {
      const nav = screen.queryAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))
      expect(nav).toBeTruthy()
      return within(nav!).getByRole('link', { name: 'Planification' })
    })
    expect(link.getAttribute('data-active')).toBe('true')
  })

  it('disables a task and runs another one now', async () => {
    setup()
    const calls: string[] = []
    server.use(
      http.put(`${SCHEDULES}/${NIGHT}`, async ({ request }) => {
        const body = (await request.json()) as Schemas['ScheduleRequest']
        calls.push(`put enabled=${body.enabled} steps=${body.steps.length}`)
        return HttpResponse.json(schedule({ enabled: body.enabled }))
      }),
      http.post(`${SCHEDULES}/${NIGHT}/run`, () => {
        calls.push('run')
        return HttpResponse.json(schedule().last_run, { status: 202 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('switch', { name: 'Activer Nuit' }))
    await user.click(screen.getByRole('button', { name: 'Exécuter maintenant' }))

    await waitFor(() => expect(calls).toEqual(['put enabled=false steps=2', 'run']))
    expect(await screen.findByText('Tâche Nuit lancée.')).toBeTruthy()
  })

  it('creates a weekly task from the shortcuts with ordered steps', async () => {
    setup([])
    let body: Schemas['ScheduleRequest'] | undefined
    server.use(
      http.post(SCHEDULES, async ({ request }) => {
        body = (await request.json()) as Schemas['ScheduleRequest']
        return HttpResponse.json(schedule({ name: body.name }), { status: 201 })
      }),
      http.get(`${SCHEDULES}/${NIGHT}`, () => HttpResponse.json(schedule())),
      http.get(`${SCHEDULES}/${NIGHT}/runs`, () => HttpResponse.json([])),
    )
    const { user } = await renderApp(`${PAGE}/new`)

    await user.type(await screen.findByLabelText('Nom'), 'Redémarrage du week-end')
    await user.click(screen.getByRole('combobox', { name: 'Fréquence' }))
    await user.click(await screen.findByRole('option', { name: 'Certains jours' }))
    // Lundi, mercredi, vendredi par défaut : on passe au samedi et dimanche.
    for (const day of ['lun.', 'mer.', 'ven.', 'sam.', 'dim.']) await user.click(screen.getByRole('checkbox', { name: day }))
    expect(await screen.findByText('Prochaines exécutions')).toBeTruthy()

    await user.click(screen.getByRole('button', { name: 'Ajouter une étape' }))
    await user.type(screen.getByLabelText('Commande'), 'say Redémarrage dans 1 min')
    await user.click(screen.getByRole('button', { name: 'Ajouter une étape' }))
    const actions = screen.getAllByRole('combobox', { name: 'Action' })
    await user.click(actions[1]!)
    await user.click(await screen.findByRole('option', { name: 'Alimentation' }))
    await user.click(screen.getByRole('button', { name: 'Monter l’étape 2' }))
    await user.click(screen.getByRole('button', { name: 'Créer la tâche' }))

    await waitFor(() =>
      expect(body).toMatchObject({
        name: 'Redémarrage du week-end',
        cron: '0 4 * * 0,6',
        enabled: true,
        only_when_online: false,
        steps: [
          { action: 'power', value: 'restart', delay_secs: 0, continue_on_failure: false },
          { action: 'command', value: 'say Redémarrage dans 1 min', delay_secs: 0, continue_on_failure: false },
        ],
      }),
    )
  })

  it('creates an upgrade step that targets the latest published version by default', async () => {
    setup([])
    let body: Schemas['ScheduleRequest'] | undefined
    server.use(
      http.post(SCHEDULES, async ({ request }) => {
        body = (await request.json()) as Schemas['ScheduleRequest']
        return HttpResponse.json(schedule({ name: body.name }), { status: 201 })
      }),
      http.get(`${SCHEDULES}/${NIGHT}`, () => HttpResponse.json(schedule())),
      http.get(`${SCHEDULES}/${NIGHT}/runs`, () => HttpResponse.json([])),
    )
    const { user } = await renderApp(`${PAGE}/new`)

    await user.type(await screen.findByLabelText('Nom'), 'Mise à jour mensuelle')
    await user.click(screen.getByRole('button', { name: 'Ajouter une étape' }))
    await user.click(screen.getByRole('combobox', { name: 'Action' }))
    await user.click(await screen.findByRole('option', { name: 'Mise à jour du prefab' }))
    expect(screen.getByPlaceholderText('Dernière version publiée')).toBeTruthy()
    await user.click(screen.getByRole('button', { name: 'Créer la tâche' }))

    await waitFor(() =>
      expect(body?.steps).toEqual([{ action: 'upgrade', value: null, delay_secs: 0, continue_on_failure: false }]),
    )
  })

  it('only offers the actions the admin holds', async () => {
    setup([])
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({
          ...profile,
          all_permissions: false,
          permissions: ['servers.read', 'servers.schedules.read', 'servers.schedules.manage', 'servers.power'],
        }),
      ),
    )
    const { user } = await renderApp(`${PAGE}/new`)

    await user.click(await screen.findByRole('button', { name: 'Ajouter une étape' }))
    await user.click(screen.getByRole('combobox', { name: 'Action' }))
    expect(await screen.findByRole('option', { name: 'Alimentation' })).toBeTruthy()
    expect(screen.queryByRole('option', { name: 'Commande console' })).toBeNull()
    expect(screen.queryByRole('option', { name: 'Sauvegarde' })).toBeNull()
  })

  it('shows the history of a task with the cause of each failure', async () => {
    setup()
    server.use(
      http.get(`${SCHEDULES}/${NIGHT}`, () => HttpResponse.json(schedule())),
      http.get(`${SCHEDULES}/${NIGHT}/runs`, () =>
        HttpResponse.json([
          { ...schedule().last_run!, id: 'r2', outcome: 'skipped', failed_step: null, reason: 'offline' },
          { ...schedule().last_run!, id: 'r1', trigger: 'manual', failed_step: 1, reason: 'server_not_running' },
        ]),
      ),
    )
    await renderApp(`${PAGE}/${NIGHT}`)

    expect(await screen.findByText('Serveur hors ligne')).toBeTruthy()
    expect(screen.getByText('Manuelle')).toBeTruthy()
    expect(screen.getByText(/^Étape 2 : /)).toBeTruthy()
    expect((screen.getByLabelText('Nom') as HTMLInputElement).value).toBe('Nuit')
  })
})
