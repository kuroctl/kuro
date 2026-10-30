// Appels API des régions (emplacement physique des nœuds). Aucune logique ici : les erreurs
// remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/regions'

export type Region = Schemas['RegionDto']

export function useRegionsApi() {
  return {
    list: () => http.get<Region[]>(BASE).then((response) => response.data),

    create: (body: Schemas['CreateRegionRequest']) => http.post<Region>(BASE, body).then((response) => response.data),

    update: (id: string, body: Schemas['UpdateRegionRequest']) =>
      http.patch<Region>(`${BASE}/${id}`, body).then((response) => response.data),

    remove: (id: string) => http.delete<void>(`${BASE}/${id}`).then(() => undefined),
  }
}
