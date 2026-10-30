// Erreurs de l'API affichées à l'admin, sur le formulaire de changement de mot de passe :
// erreur de champ, toast traduit avec paramètres, réseau coupé, code inconnu.
import type { UserEvent } from '@testing-library/user-event'
import { screen } from '@testing-library/vue'
import { http, HttpResponse } from 'msw'
import { describe, expect, it } from 'vitest'

import { API, problem, server, signedIn } from '../support/api'
import { renderApp } from '../support/app'

async function changePassword(user: UserEvent) {
  await user.type(await screen.findByLabelText('Mot de passe actuel'), 'old password value')
  await user.type(screen.getByLabelText('Nouveau mot de passe'), 'short')
  await user.click(screen.getByRole('button', { name: 'Enregistrer' }))
}

async function submitWith(response: () => Response) {
  server.use(...signedIn(), http.put(`${API}/me/password`, response))
  const { user } = await renderApp('/account/security')
  await changePassword(user)
}

describe('API errors', () => {
  it('shows a field error under its field', async () => {
    await submitWith(() =>
      problem(422, 'validation_failed', { fields: [{ field: 'new_password', code: 'weak_password' }] }),
    )

    expect(await screen.findByText('Le mot de passe doit contenir au moins 12 caractères.')).toBeTruthy()
    expect(screen.queryByText('Certains champs sont invalides.')).toBeNull()
  })

  it('interpolates the error params in the toast', async () => {
    await submitWith(() => problem(403, 'forbidden', { params: { permission: 'iam.users.write' } }))

    expect(await screen.findByText('Vous n’avez pas la permission « iam.users.write ».')).toBeTruthy()
    expect(screen.getByText('Référence : 01234567')).toBeTruthy()
  })

  it('reports an unreachable server', async () => {
    await submitWith(() => HttpResponse.error())

    expect(await screen.findByText('Impossible de joindre le serveur.')).toBeTruthy()
  })

  it('falls back to a generic message for a code the panel does not know yet', async () => {
    await submitWith(() => problem(409, 'node_quarantined'))

    expect(await screen.findByText('Une erreur inattendue est survenue.')).toBeTruthy()
    expect(screen.getByText('Référence : 01234567')).toBeTruthy()
  })

  it('confirms a successful change', async () => {
    await submitWith(() => new HttpResponse(null, { status: 204 }))

    expect(await screen.findByText('Mot de passe modifié. Vos autres sessions ont été fermées.')).toBeTruthy()
  })
})
