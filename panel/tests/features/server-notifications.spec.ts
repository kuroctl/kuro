// Notifications d'un serveur (ADR-0044) : abonnements listés avec leur destination, création
// avec une destination propre, canal partagé proposé seulement quand l'API l'autorise, journal.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const PAGE = `/servers/${SERVER_ID}/notifications`
const NOTIFICATIONS = `${API}/servers/${SERVER_ID}/notifications`

const subscription: Schemas['SubscriptionDto'] = {
  id: '01a19200-0000-7000-8000-00000000a001',
  name: 'Mon Discord',
  events: ['server.crashed', 'server.backup_failed'],
  enabled: true,
  owned: true,
  channel: {
    id: '01a19200-0000-7000-8000-00000000c001',
    name: 'Mon Discord',
    kind: 'discord',
    url_hint: 'https://discord.com/api/webhook…',
    has_secret: false,
    language: 'fr',
    enabled: true,
    created_at: '2026-10-30T10:00:00Z',
    updated_at: '2026-10-30T10:00:00Z',
  },
}

function setup(sharedAllowed = false, subscriptions = [subscription]) {
  server.use(
    ...signedIn(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto())),
    http.get(NOTIFICATIONS, () =>
      HttpResponse.json({
        events: ['server.crashed', 'server.schedule_failed', 'server.backup_failed', 'server.upgrade_failed', 'server.disk_high'],
        shared_allowed: sharedAllowed,
        subscriptions,
      }),
    ),
    http.get(`${NOTIFICATIONS}/deliveries`, () => HttpResponse.json([])),
    http.get(`${API}/settings/notifications/channels`, () =>
      HttpResponse.json([{ ...subscription.channel, id: '01a19200-0000-7000-8000-00000000c002', name: 'Équipe', kind: 'slack' }]),
    ),
    ...catalog(),
  )
}

describe('server notifications', () => {
  it('lists the subscriptions with their destination and events', async () => {
    setup()
    await renderApp(PAGE)

    const row = (await screen.findAllByText('Mon Discord')).map((e) => e.closest<HTMLElement>('[data-subscription]')).find(Boolean)!
    expect(within(row).getByText('https://discord.com/api/webhook…')).toBeTruthy()
    expect(within(row).getByText('Sauvegarde en échec')).toBeTruthy()
    await waitFor(() => {
      const nav = screen.queryAllByRole('navigation').find((n) => within(n).queryByText('Vue d’ensemble'))
      expect(nav).toBeTruthy()
      expect(within(nav!).getByRole('link', { name: 'Notifications' })).toBeTruthy()
    })
  })

  it('creates a subscription with its own destination, without offering shared channels', async () => {
    setup(false, [])
    let body: Schemas['SubscriptionRequest'] | undefined
    server.use(
      http.post(NOTIFICATIONS, async ({ request }) => {
        body = (await request.json()) as Schemas['SubscriptionRequest']
        return HttpResponse.json({ ...subscription, name: body.name }, { status: 201 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('button', { name: 'Ajouter un abonnement' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).queryByLabelText('Destination')).toBeNull()
    await user.type(within(dialog).getByLabelText('Nom'), 'Alertes')
    await user.click(within(dialog).getByRole('checkbox', { name: 'Serveur arrêté de façon inattendue' }))
    await user.type(within(dialog).getByLabelText('URL du webhook'), 'https://hooks.test/mine')
    await user.click(within(dialog).getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({ name: 'Alertes', events: ['server.crashed'], enabled: true, kind: 'webhook', url: 'https://hooks.test/mine', language: 'fr' }),
    )
    expect(await screen.findByText('Abonnement enregistré.')).toBeTruthy()
  })

  it('offers the shared channels when the API allows it', async () => {
    setup(true, [])
    let body: Schemas['SubscriptionRequest'] | undefined
    server.use(
      http.post(NOTIFICATIONS, async ({ request }) => {
        body = (await request.json()) as Schemas['SubscriptionRequest']
        return HttpResponse.json({ ...subscription, owned: false, name: body.name }, { status: 201 })
      }),
    )
    const { user } = await renderApp(PAGE)

    await user.click(await screen.findByRole('button', { name: 'Ajouter un abonnement' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Nom'), 'Vers l’équipe')
    await user.click(within(dialog).getByRole('checkbox', { name: 'Disque presque plein' }))
    await user.click(within(dialog).getByRole('combobox', { name: 'Destination' }))
    await user.click(await screen.findByRole('option', { name: 'Canal partagé des paramètres' }))
    await user.click(within(dialog).getByRole('combobox', { name: 'Canaux' }))
    await user.click(await screen.findByRole('option', { name: 'Équipe' }))
    await user.click(within(dialog).getByRole('button', { name: 'Enregistrer' }))

    await waitFor(() =>
      expect(body).toEqual({ name: 'Vers l’équipe', events: ['server.disk_high'], enabled: true, channel_id: '01a19200-0000-7000-8000-00000000c002' }),
    )
  })
})
