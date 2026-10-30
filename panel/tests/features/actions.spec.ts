// Actions d'un prefab (ADR-0039) : carte sur la page du serveur, dialogue des paramètres, résultat,
// refus d'un paramètre sous son champ, tâche préremplie pour une action planifiable.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, localeFr, manifest, prefabSummary, SERVER_ID, serverDto } from '../support/servers'

const withActions = {
  ...manifest,
  actions: {
    'save-all': { service: 'server', command: ['rcon-cli', 'save-all'], schedulable: true },
    'whitelist-add': {
      service: 'server',
      command: ['rcon-cli', 'whitelist', 'add', '{{ params.player }}'],
      params: { player: { type: 'string', pattern: '^[A-Za-z0-9_]{3,16}$', required: true } },
      timeout: 15,
    },
  },
}
const localeWithActions = {
  ...localeFr,
  actions: {
    'save-all': { label: 'Sauvegarder le monde', help: 'Écrit le monde sur le disque.' },
    'whitelist-add': { label: 'Ajouter à la liste blanche', params: { player: { label: 'Joueur' } } },
  },
}

/** Vue d'ensemble d'un serveur en marche dont le prefab déclare deux actions. */
function page() {
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers`, () => HttpResponse.json([serverDto({ state: 'running' })])),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
  )
  // Surcharges du catalogue : dans un appel à part, pour passer devant ses gestionnaires.
  server.use(
    http.get(`${API}/prefabs/minecraft-java/1.0.0`, () =>
      HttpResponse.json({ ...prefabSummary, manifest: withActions, locales: ['en', 'fr'] }),
    ),
    http.get(`${API}/prefabs/minecraft-java/1.0.0/locales/fr`, () => HttpResponse.json(localeWithActions)),
  )
}

describe('prefab actions', () => {
  it('runs an action with its parameter and shows the result', async () => {
    page()
    const bodies: unknown[] = []
    server.use(
      http.post(`${API}/servers/${SERVER_ID}/actions/whitelist-add`, async ({ request }) => {
        bodies.push(await request.json())
        return HttpResponse.json({ exit_code: 0, output: 'Added Steve to the whitelist\n', truncated: false })
      }),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    await user.click(await screen.findByRole('button', { name: 'Ajouter à la liste blanche' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Joueur'), 'Steve')
    await user.click(within(dialog).getByRole('button', { name: 'Lancer' }))

    await waitFor(() => expect(bodies).toHaveLength(1))
    expect(bodies[0]).toEqual({ params: { player: 'Steve' } })
    expect(await within(dialog).findByText('Added Steve to the whitelist')).toBeTruthy()
    expect(within(dialog).getByText('Code de sortie : 0')).toBeTruthy()
  })

  it('shows a refused parameter under its field and the schedule link of a schedulable action', async () => {
    page()
    server.use(
      http.post(`${API}/servers/${SERVER_ID}/actions/whitelist-add`, () =>
        problem(422, 'validation_failed', { fields: [{ field: 'params.player', code: 'invalid_action_param' }] }),
      ),
    )
    const { user } = await renderApp(`/servers/${SERVER_ID}`)

    // Seule l'action planifiable propose une tâche.
    const schedule = await screen.findByRole('link', { name: 'Planifier Sauvegarder le monde' })
    expect(schedule.getAttribute('href')).toBe(`/servers/${SERVER_ID}/schedules/new?action=save-all`)
    expect(screen.queryByRole('link', { name: /Planifier Ajouter/ })).toBeNull()

    await user.click(screen.getByRole('button', { name: 'Ajouter à la liste blanche' }))
    const dialog = await screen.findByRole('dialog')
    await user.type(within(dialog).getByLabelText('Joueur'), 'x')
    await user.click(within(dialog).getByRole('button', { name: 'Lancer' }))

    expect(await within(dialog).findByText(/Paramètre invalide/)).toBeTruthy()
    expect(within(dialog).getByLabelText('Joueur').getAttribute('aria-invalid')).toBe('true')
  })
})
