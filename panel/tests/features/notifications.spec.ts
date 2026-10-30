// Notifications et webhooks (ADR-0044) : canaux (URL et secret envoyés une fois, jamais relus,
// test), règles (événements et canaux cochés), journal des envois.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, node, serverDto } from '../support/servers'

const NOTIFICATIONS = `${API}/settings/notifications`
const CHANNEL_ID = '01a19100-0000-7000-8000-00000000c001'

const channel: Schemas['ChannelDto'] = {
  id: CHANNEL_ID,
  name: 'Ops',
  kind: 'webhook',
  url_hint: 'https://hooks.test/kuro',
  has_secret: true,
  language: 'fr',
  enabled: true,
  created_at: '2026-10-30T10:00:00Z',
  updated_at: '2026-10-30T10:00:00Z',
}

const rule: Schemas['RuleDto'] = {
  id: '01a19100-0000-7000-8000-00000000a001',
  name: 'Tout',
  scope: 'all',
  node_id: null,
  server_id: null,
  events: ['server.crashed'],
  channels: [CHANNEL_ID],
  enabled: true,
  created_at: '2026-10-30T10:00:00Z',
  updated_at: '2026-10-30T10:00:00Z',
}

const delivery: Schemas['DeliveryDto'] = {
  id: '01a19100-0000-7000-8000-00000000d001',
  rule_id: rule.id,
  channel_id: CHANNEL_ID,
  event: { kind: 'server.crashed', at: '2026-10-30T11:00:00Z', server: { name: 'Survie' }, reason: 'exit_code_137' },
  status: 'sent',
  attempts: 1,
  next_attempt_at: '2026-10-30T11:00:00Z',
  last_error: null,
  created_at: '2026-10-30T11:00:00Z',
  sent_at: '2026-10-30T11:00:01Z',
}

function setup(channels = [channel], rules = [rule], deliveries = [delivery]) {
  server.use(
    ...signedIn(),
    http.get(`${NOTIFICATIONS}/events`, () =>
      HttpResponse.json(['server.crashed', 'server.schedule_failed', 'server.backup_failed', 'server.upgrade_failed', 'server.disk_high', 'node.offline', 'node.online']),
    ),
    http.get(`${NOTIFICATIONS}/channels`, () => HttpResponse.json(channels)),
    http.get(`${NOTIFICATIONS}/rules`, () => HttpResponse.json(rules)),
    http.get(`${NOTIFICATIONS}/deliveries`, () => HttpResponse.json(deliveries)),
    http.get(`${API}/nodes`, () => HttpResponse.json([node])),
    http.get(`${API}/servers`, () => HttpResponse.json([serverDto])),
    ...catalog(),
  )
}

describe('notifications', () => {
  it('lists channels, rules and the delivery log', async () => {
    setup()
    await renderApp('/settings/notifications')

    const row = (await screen.findAllByText('Ops')).map((e) => e.closest<HTMLElement>('[data-channel]')).find(Boolean)!
    expect(within(row).getByText('https://hooks.test/kuro')).toBeTruthy()
    expect(within(row).getByText('Signé')).toBeTruthy()
    const ruleRow = (await screen.findAllByText('Tout')).map((e) => e.closest<HTMLElement>('[data-rule]')).find(Boolean)!
    expect(within(ruleRow).getByText('Serveur arrêté de façon inattendue')).toBeTruthy()
    const log = (await screen.findByText('Survie')).closest<HTMLElement>('[data-delivery]')!
    expect(within(log).getByText('Envoyé')).toBeTruthy()
  })

  it('creates a channel, sending the URL and the secret once', async () => {
    setup([])
    let body: Schemas['ChannelRequest'] | undefined
    server.use(
      http.post(`${NOTIFICATIONS}/channels`, async ({ request }) => {
        body = (await request.json()) as Schemas['ChannelRequest']
        return HttpResponse.json({ ...channel, name: body.name }, { status: 201 })
      }),
    )
    const { user } = await renderApp('/settings/notifications')

    await user.click(await screen.findByRole('button', { name: 'Ajouter un canal' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Discord ops')
    await user.type(within(dialog).getByLabelText('URL du webhook'), 'https://hooks.test/new')
    await user.type(within(dialog).getByLabelText('Secret HMAC (facultatif)'), 's3cret')
    await user.click(within(dialog).getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({ name: 'Discord ops', kind: 'webhook', url: 'https://hooks.test/new', secret: 's3cret', language: 'fr', enabled: true }),
    )
    expect(await screen.findByText('Canal enregistré.')).toBeTruthy()
  })

  it('tests a channel and reports the answer', async () => {
    setup()
    let tested = false
    server.use(
      http.post(`${NOTIFICATIONS}/channels/${CHANNEL_ID}/test`, () => {
        tested = true
        return HttpResponse.json({ ok: true, status: 204, error: null })
      }),
    )
    const { user } = await renderApp('/settings/notifications')

    await user.click(await screen.findByRole('button', { name: 'Tester' }))

    expect(await screen.findByText('Test envoyé sur Ops.')).toBeTruthy()
    expect(tested).toBe(true)
  })

  it('creates a rule from checked events and channels', async () => {
    setup()
    let body: Schemas['RuleRequest'] | undefined
    server.use(
      http.post(`${NOTIFICATIONS}/rules`, async ({ request }) => {
        body = (await request.json()) as Schemas['RuleRequest']
        return HttpResponse.json({ ...rule, id: '01a19100-0000-7000-8000-00000000a002', name: body.name }, { status: 201 })
      }),
    )
    const { user } = await renderApp('/settings/notifications')

    await user.click(await screen.findByRole('button', { name: 'Ajouter une règle' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Nœuds')
    await user.click(within(dialog).getByRole('checkbox', { name: 'Nœud hors ligne' }))
    await user.click(within(dialog).getByRole('checkbox', { name: 'Ops' }))
    await user.click(within(dialog).getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({ name: 'Nœuds', scope: 'all', node_id: null, server_id: null, events: ['node.offline'], channels: [CHANNEL_ID], enabled: true }),
    )
  })
})
