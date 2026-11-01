// Accès SFTP (ADR-0047) : clés SSH du compte, connexion d'un serveur, bascule du projet.
import { screen, waitFor, within } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import type { Schemas } from '@/shared/api/types'

import { API, problem, profile, server, sftpDisabled, signedIn } from '../support/api'
import { renderApp } from '../support/app'
import { catalog, SERVER_ID, serverDto } from '../support/servers'

const KEY: Schemas['SshKeyDto'] = {
  id: '01a17b00-0000-7000-8000-000000000001',
  name: 'laptop',
  algorithm: 'ssh-ed25519',
  fingerprint: 'SHA256:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG',
  created_at: '2026-10-30T10:00:00Z',
  last_used_at: null,
}

const settings: Schemas['SettingsDto'] = {
  default_memory_mb: 2048,
  max_memory_mb: null,
  default_cpu_percent: 200,
  max_cpu_percent: null,
  default_disk_mb: 10240,
  max_disk_mb: null,
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
  mcp_url: null,
  updated_at: '2026-10-27T10:00:00Z',
}

function filesPage() {
  server.use(
    ...catalog(),
    http.get(`${API}/servers/${SERVER_ID}`, () => HttpResponse.json(serverDto({ state: 'running' }))),
    http.get(`${API}/servers/${SERVER_ID}/files`, () => HttpResponse.json({ path: '/', entries: [] })),
  )
}

describe('ssh keys', () => {
  it('lists the keys of the account and adds one', async () => {
    let body: unknown
    let keys: Schemas['SshKeyDto'][] = []
    server.use(
      http.get(`${API}/me/ssh-keys`, () => HttpResponse.json(keys)),
      http.post(`${API}/me/ssh-keys`, async ({ request }) => {
        body = await request.json()
        keys = [KEY]
        return HttpResponse.json(KEY, { status: 201 })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    expect(await screen.findByText('Aucune clé enregistrée.')).toBeTruthy()
    await user.type(screen.getByLabelText('Nom de la clé'), 'laptop')
    await user.type(screen.getByLabelText('Clé publique'), 'ssh-ed25519 AAAA me@laptop')
    await user.click(screen.getByRole('button', { name: 'Ajouter la clé' }))

    await waitFor(() => expect(body).toEqual({ name: 'laptop', public_key: 'ssh-ed25519 AAAA me@laptop' }))
    expect(await screen.findByText('Clé ajoutée.')).toBeTruthy()
    expect(await screen.findByText(KEY.fingerprint)).toBeTruthy()
  })

  it('shows a refused key under its field', async () => {
    server.use(
      http.post(`${API}/me/ssh-keys`, () =>
        problem(422, 'ssh_key_unsupported', {
          fields: [{ field: 'public_key', code: 'ssh_key_unsupported', params: {} }],
        }),
      ),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    await user.type(await screen.findByLabelText('Nom de la clé'), 'old')
    await user.type(screen.getByLabelText('Clé publique'), 'ssh-rsa AAAA me')
    await user.click(screen.getByRole('button', { name: 'Ajouter la clé' }))

    expect(await screen.findByText('Type de clé non accepté : utilisez une clé ed25519 ou ECDSA.')).toBeTruthy()
  })

  it('removes a key after confirmation', async () => {
    let removed = false
    server.use(
      http.get(`${API}/me/ssh-keys`, () => HttpResponse.json(removed ? [] : [KEY])),
      http.delete(`${API}/me/ssh-keys/${KEY.id}`, () => {
        removed = true
        return new HttpResponse(null, { status: 204 })
      }),
      ...signedIn(),
    )
    const { user } = await renderApp('/account/security')

    await user.click(await screen.findByRole('button', { name: 'Retirer la clé laptop' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText('Retirer la clé « laptop » ?')).toBeTruthy()
    await user.click(within(dialog).getByRole('button', { name: 'Retirer' }))

    expect(await screen.findByText('Clé laptop retirée.')).toBeTruthy()
    expect(removed).toBe(true)
  })
})

describe('sftp connection', () => {
  it('shows the host, port, username and host key of the node', async () => {
    filesPage()
    server.use(
      http.get(`${API}/servers/${SERVER_ID}/sftp`, () => HttpResponse.json({ ...sftpDisabled, enabled: true })),
      ...signedIn(),
    )
    const { user, router } = await renderApp(`/servers/${SERVER_ID}/files`)

    // Onglet à part, gardé dans l'adresse.
    await user.click(await screen.findByRole('tab', { name: 'SFTP' }))
    await waitFor(() => expect(router.currentRoute.value.query.tab).toBe('sftp'))
    expect(await screen.findByText('Accès SFTP')).toBeTruthy()
    expect(await screen.findByText('node-1.nodes.play.example.com')).toBeTruthy()
    expect(screen.getByText('2022')).toBeTruthy()
    expect(screen.getByText('a1b2c3d4e5.f6a7b8c9d0')).toBeTruthy()
    expect(screen.getByText(sftpDisabled.host_key_fingerprint!)).toBeTruthy()
    const open = screen.getByRole('link', { name: 'Ouvrir dans le client SFTP' })
    expect(open.getAttribute('href')).toBe('sftp://a1b2c3d4e5.f6a7b8c9d0@node-1.nodes.play.example.com:2022')
  })

  it('says when SFTP is switched off in the project', async () => {
    filesPage()
    server.use(...signedIn())
    await renderApp(`/servers/${SERVER_ID}/files?tab=sftp`)

    expect(await screen.findByText('Le SFTP est coupé dans les paramètres du projet.')).toBeTruthy()
  })

  it('is hidden without the sftp permission', async () => {
    let asked = false
    filesPage()
    server.use(...signedIn())
    // Second `server.use` : il passe devant le profil de `signedIn()`.
    server.use(
      http.get(`${API}/me`, () =>
        HttpResponse.json({ ...profile, all_permissions: false, permissions: ['servers.read', 'servers.files.read'] }),
      ),
      http.get(`${API}/servers/${SERVER_ID}/sftp`, () => {
        asked = true
        return HttpResponse.json(sftpDisabled)
      }),
    )
    await renderApp(`/servers/${SERVER_ID}/files?tab=sftp`)

    expect(await screen.findByText('Fichiers')).toBeTruthy()
    expect(screen.queryByRole('tab', { name: 'SFTP' })).toBeNull()
    expect(screen.queryByText('Accès SFTP')).toBeNull()
    expect(asked).toBe(false)
  })
})

describe('sftp switch', () => {
  it('opens SFTP for the whole project', async () => {
    let body: unknown
    server.use(
      ...signedIn(),
      http.get(`${API}/settings`, () => HttpResponse.json(settings)),
      http.patch(`${API}/settings`, async ({ request }) => {
        body = await request.json()
        return HttpResponse.json({ ...settings, sftp_enabled: true })
      }),
    )
    const { user } = await renderApp('/settings')

    await user.click(await screen.findByRole('switch', { name: 'SFTP ouvert' }))

    await waitFor(() => expect(body).toEqual({ sftp_enabled: true }))
    expect(await screen.findByText('SFTP ouvert.')).toBeTruthy()
  })

  it('shows everything it knows when the node has no address', async () => {
    filesPage()
    server.use(
      http.get(`${API}/servers/${SERVER_ID}/sftp`, () =>
        HttpResponse.json({ ...sftpDisabled, enabled: true, host: null }),
      ),
      ...signedIn(),
    )
    await renderApp(`/servers/${SERVER_ID}/files?tab=sftp`)

    // Tout le reste est montré ; seule la ligne de l'hôte signale le manque.
    expect(await screen.findByText(/Adresse publique de node-1 non renseignée/)).toBeTruthy()
    expect(screen.getByText('2022')).toBeTruthy()
    expect(screen.getByText('a1b2c3d4e5.f6a7b8c9d0')).toBeTruthy()
    expect(screen.queryByRole('link', { name: 'Ouvrir dans le client SFTP' })).toBeNull()
  })
})
