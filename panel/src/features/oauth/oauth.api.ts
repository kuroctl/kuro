// Appels API des agents IA (MCP, ADR-0023) : consentement et applications connectées. Aucune
// logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/oauth'

export type ConsentRequest = Schemas['ConsentRequestDto']
export type Grant = Schemas['GrantDto']

export function useOAuthApi() {
  return {
    request: (id: string) => http.get<ConsentRequest>(`${BASE}/requests/${id}`).then((response) => response.data),

    /** `scopes` : droits retenus ; rend l'URI de retour du client, avec le code. */
    approve: (id: string, scopes: string[]) =>
      http.post<Schemas['ConsentRedirectDto']>(`${BASE}/requests/${id}/approve`, { scopes }).then((response) => response.data.redirect_to),

    deny: (id: string) =>
      http.post<Schemas['ConsentRedirectDto']>(`${BASE}/requests/${id}/deny`).then((response) => response.data.redirect_to),

    grants: () => http.get<Grant[]>(`${BASE}/grants`).then((response) => response.data),

    revokeGrant: (id: string) => http.delete(`${BASE}/grants/${id}`),
  }
}
