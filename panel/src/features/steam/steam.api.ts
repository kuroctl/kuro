// Appels API du compte Steam de l'installation (ADR-0033). Aucune logique ici : les erreurs
// remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/settings/steam-account'

export type SteamAccount = Schemas['SteamAccountDto']

export function useSteamApi() {
  return {
    get: () => http.get<SteamAccount>(BASE).then((response) => response.data),

    /** Le mot de passe est chiffré par l'API et jamais renvoyé. */
    set: (username: string, password: string) => http.put<void>(BASE, { username, password }).then(() => undefined),

    remove: () => http.delete<void>(BASE).then(() => undefined),
  }
}
