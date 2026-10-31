// Appels API de la première mise en route (ADR-0045), publics. Aucune logique ici : les erreurs
// remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

export type SetupStatus = Schemas['SetupStatusDto']
export type SetupRequest = Schemas['SetupRequestDto']

export function useSetupApi() {
  return {
    status: () => http.get<SetupStatus>('/setup').then((response) => response.data),

    run: (body: SetupRequest) => http.post<Schemas['SetupDoneDto']>('/setup', body).then((response) => response.data),
  }
}
