// Appels API des paramètres du projet (ADR-0019). Aucune logique ici : les erreurs remontent à
// l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/settings'

export type PlatformSettings = Schemas['SettingsDto']
export type SettingsPatch = Schemas['UpdatePlatformSettingsRequest']

export function useSettingsApi() {
  return {
    get: () => http.get<PlatformSettings>(BASE).then((response) => response.data),

    update: (patch: SettingsPatch) => http.patch<PlatformSettings>(BASE, patch).then((response) => response.data),
  }
}
