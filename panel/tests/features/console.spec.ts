// Console d'un serveur : ticket puis WebSocket, sortie et stats en direct, commandes.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse, ws } from 'msw'
import { describe, expect, it, vi } from 'vitest'

import { API, problem, profile, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

vi.mock('@xterm/xterm', () => import('../support/xterm'))
vi.mock('@xterm/addon-fit', () => import('../support/xterm'))

const CONSOLE = `/servers/${SERVER_ID}/console`

/** WebSocket de console simulé : enregistre les messages reçus, laisse le test répondre. */
function consoleSocket() {
  const link = ws.link(`ws://${window.location.host}/api/admin/v1/servers/:id/console`)
  const received: unknown[] = []
  const sockets: { send: (data: unknown) => void; close: (code: number) => void }[] = []
  const handler = link.addEventListener('connection', ({ client }) => {
    client.addEventListener('message', (event) => {
      received.push(JSON.parse(String(event.data)))
    })
    sockets.push({
      send: (data) => client.send(JSON.stringify(data)),
      close: (code) => client.close(code, 'test'),
    })
  })
  return {
    handler,
    received,
    /** Dernière connexion ouverte par le panel. */
    get current() {
      const socket = sockets.at(-1)
      if (!socket) throw new Error('no console connection')
      return socket
    },
    get count() {
      return sockets.length
    },
  }
}

function tickets() {
  let issued = 0
  return {
    handler: http.post(`${API}/servers/${SERVER_ID}/console/ticket`, () => {
      issued += 1
      return HttpResponse.json({ ticket: `kct_${issued}`, expires_in_secs: 30 }, { status: 201 })
    }),
    get issued() {
      return issued
    },
  }
}

function setup(state: 'running' | 'offline' = 'running') {
  const socket = consoleSocket()
  const ticket = tickets()
  server.use(
    ...signedIn(),
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state }))),
    ticket.handler,
    socket.handler,
  )
  return { socket, ticket }
}

describe('server console', () => {
  it('authenticates with a ticket sent as the first message, never in the URL', async () => {
    const { socket } = setup()
    await renderApp(CONSOLE)

    await waitFor(() => expect(socket.received).toEqual([{ type: 'auth', ticket: 'kct_1' }]))
  })

  it('shows the live output and consumption of the server', async () => {
    const { socket } = setup()
    await renderApp(CONSOLE)
    await waitFor(() => expect(socket.received).toHaveLength(1))

    socket.current.send({ type: 'lines', lines: [{ at_ms: 1, text: 'Done (3.2s)!' }] })
    socket.current.send({
      type: 'stats',
      cpu_millis: 750,
      memory_bytes: 1024 * 1024 * 1024,
      memory_limit_bytes: 2 * 1024 * 1024 * 1024,
      network_rx_bytes: 0,
      network_tx_bytes: 0,
    })

    await waitFor(() => expect(screen.getByTestId('terminal').textContent).toContain('Done (3.2s)!'))
    expect(await screen.findByText('75 %', { exact: false })).toBeTruthy()
    expect(screen.getByText('sur 2 cœurs')).toBeTruthy()
    expect(screen.getByText('1 Go', { exact: false })).toBeTruthy()
    expect(screen.getByRole('status').textContent).toContain('En direct')
  })

  it('tells when lines were lost', async () => {
    const { socket } = setup()
    await renderApp(CONSOLE)
    await waitFor(() => expect(socket.received).toHaveLength(1))

    socket.current.send({ type: 'lines', lines: [], dropped: 12 })

    await waitFor(() => expect(screen.getByTestId('terminal').textContent).toContain('12 lignes perdues'))
  })

  it('follows the server state live', async () => {
    const { socket } = setup()
    await renderApp(CONSOLE)
    await waitFor(() => expect(socket.received).toHaveLength(1))

    socket.current.send({ type: 'state', state: 'stopping' })

    expect(await screen.findByText('Arrêt')).toBeTruthy()
  })

  it('reconnects with a new ticket when the session expires', async () => {
    const { socket, ticket } = setup()
    await renderApp(CONSOLE)
    await waitFor(() => expect(socket.count).toBe(1))

    socket.current.close(4408)

    await waitFor(() => expect(socket.count).toBe(2))
    expect(ticket.issued).toBe(2)
    await waitFor(() => expect(socket.received.at(-1)).toEqual({ type: 'auth', ticket: 'kct_2' }))
  })

  it('sends a command and recalls it from the history', async () => {
    let body: unknown
    const { socket } = setup()
    server.use(
      http.post(`${API}/servers/${SERVER_ID}/command`, async ({ request }) => {
        body = await request.json()
        return new HttpResponse(null, { status: 202 })
      }),
    )
    const { user } = await renderApp(CONSOLE)
    await waitFor(() => expect(socket.count).toBe(1))

    const input = screen.getByRole('textbox', { name: 'Commande' })
    await user.type(input, 'op alice{Enter}')

    await waitFor(() => expect(body).toEqual({ line: 'op alice' }))
    await waitFor(() => expect((input as HTMLInputElement).value).toBe(''))
    await user.keyboard('{ArrowUp}')
    expect((input as HTMLInputElement).value).toBe('op alice')
  })

  it('shows a refused command under the field', async () => {
    const { socket } = setup()
    server.use(
      http.post(`${API}/servers/${SERVER_ID}/command`, () =>
        problem(422, 'validation_failed', { fields: [{ field: 'line', code: 'invalid_command' }] }),
      ),
    )
    const { user } = await renderApp(CONSOLE)
    await waitFor(() => expect(socket.count).toBe(1))

    await user.type(screen.getByRole('textbox', { name: 'Commande' }), 'say hi{Enter}')

    expect(await screen.findByText(/Une commande tient sur une ligne/)).toBeTruthy()
  })

  it('accepts no command while the server is stopped', async () => {
    const { socket } = setup('offline')
    await renderApp(CONSOLE)
    await waitFor(() => expect(socket.count).toBe(1))

    const input = screen.getByRole('textbox', { name: 'Commande' })

    expect(input.hasAttribute('disabled')).toBe(true)
    expect(input.getAttribute('placeholder')).toBe('Le serveur doit être en ligne pour recevoir des commandes.')
  })

  it('stops trying without the console permission', async () => {
    const socket = consoleSocket()
    server.use(
      ...signedIn(),
      ...catalog(),
      http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
      http.post(`${API}/servers/${SERVER_ID}/console/ticket`, () =>
        problem(403, 'forbidden', { params: { permission: 'servers.console.read' } }),
      ),
      socket.handler,
    )
    await renderApp(CONSOLE)

    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('Indisponible'))
    expect(socket.count).toBe(0)
  })

  it('offers the power actions, but neither reinstall nor delete', async () => {
    setup('offline')
    await renderApp(CONSOLE)

    expect(await screen.findByRole('button', { name: 'Démarrer' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Forcer l’arrêt' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Réinstaller' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Supprimer' })).toBeNull()
  })
})

describe('server overview console', () => {
  it('shows the live output, read-only', async () => {
    const { socket } = setup()
    await renderApp(`/servers/${SERVER_ID}`)
    // Le ticket voyage dans le premier message ; un rendu lent peut en demander un second.
    await waitFor(() => expect(socket.received.at(-1)).toMatchObject({ type: 'auth' }))

    socket.current.send({ type: 'lines', lines: [{ at_ms: 1, text: 'Server started' }] })

    await waitFor(() => expect(screen.getByTestId('terminal').textContent).toContain('Server started'))
    expect(screen.queryByRole('textbox', { name: 'Commande' })).toBeNull()
    const link = screen.getByRole('link', { name: /Ouvrir la console/ })
    expect(link.getAttribute('href')).toBe(CONSOLE)
    expect(within(screen.getByRole('main')).getByRole('button', { name: 'Supprimer' })).toBeTruthy()
  })

  it('is hidden without the console permission', async () => {
    const { socket, ticket } = setup()
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read', 'servers.power'] }),
      ),
    )
    await renderApp(`/servers/${SERVER_ID}`)

    expect(await screen.findByRole('button', { name: 'Démarrer' })).toBeTruthy()
    expect(screen.queryByTestId('terminal')).toBeNull()
    expect(ticket.issued).toBe(0)
    expect(socket.count).toBe(0)
  })
})
