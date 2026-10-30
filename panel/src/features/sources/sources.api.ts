// Appels API des sources de contenu à clé d'API (ADR-0029). Aucune logique ici : les erreurs
// remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/settings/content-sources'

export type ContentSource = Schemas['ContentSourceDto']

export function useSourcesApi() {
  return {
    list: () => http.get<ContentSource[]>(BASE).then((response) => response.data),

    /** La clé est vérifiée auprès de la source avant d'être enregistrée ; jamais renvoyée. */
    setKey: (provider: string, api_key: string) => http.put<void>(`${BASE}/${provider}`, { api_key }).then(() => undefined),

    removeKey: (provider: string) => http.delete<void>(`${BASE}/${provider}`).then(() => undefined),
  }
}
