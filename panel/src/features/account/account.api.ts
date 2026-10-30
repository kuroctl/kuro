// Appels API du compte connecté. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

export function useAccountApi() {
  return {
    me: () => http.get<Schemas['ProfileDto']>('/admin/v1/me').then((response) => response.data),

    changePassword: (body: Schemas['ChangePasswordRequest']) =>
      http.put<void>('/admin/v1/me/password', body).then(() => undefined),
  }
}
