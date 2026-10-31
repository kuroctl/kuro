// Appels API des comptes admin (ADR-0007). Aucune logique ici : les erreurs remontent à
// l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/users'

export type AdminUser = Schemas['AdminUserDto']

export function useUsersApi() {
  return {
    list: () => http.get<AdminUser[]>(BASE).then((response) => response.data),

    get: (id: string) => http.get<AdminUser>(`${BASE}/${id}`).then((response) => response.data),

    create: (body: Schemas['CreateUserRequest']) => http.post<AdminUser>(BASE, body).then((response) => response.data),

    update: (id: string, body: Schemas['UpdateUserRequest']) =>
      http.patch<AdminUser>(`${BASE}/${id}`, body).then((response) => response.data),

    remove: (id: string) => http.delete<void>(`${BASE}/${id}`).then(() => undefined),

    setRoles: (id: string, role_ids: string[]) =>
      http.put<AdminUser>(`${BASE}/${id}/roles`, { role_ids }).then((response) => response.data),

    /** Envoie ou renvoie l'invitation par e-mail (ADR-0046). */
    invite: (id: string) => http.post<void>(`${BASE}/${id}/invite`).then(() => undefined),

    resetPassword: (id: string, password: string) =>
      http.put<void>(`${BASE}/${id}/password`, { password }).then(() => undefined),

    resetTotp: (id: string) => http.delete<void>(`${BASE}/${id}/totp`).then(() => undefined),

    revokeSessions: (id: string) =>
      http.delete<Schemas['RevokedSessionsResponse']>(`${BASE}/${id}/sessions`).then((response) => response.data),
  }
}
