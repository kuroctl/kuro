// Appels API de l'authentification admin. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/auth'

export function useAuthApi() {
  return {
    login: (body: Schemas['LoginRequest']) =>
      http.post<Schemas['LoginResponse']>(`${BASE}/login`, body).then((response) => response.data),

    /** Mot de passe choisi par un autre admin, remplacé avant le TOTP : même réponse que `login`. */
    changeRequiredPassword: (body: Schemas['RequiredPasswordRequest']) =>
      http.post<Schemas['LoginResponse']>(`${BASE}/password`, body).then((response) => response.data),

    verifyTotp: (body: Schemas['TotpCodeRequest']) =>
      http
        .post<Schemas['AccessTokenResponse']>(`${BASE}/totp/verify`, body)
        .then((response) => response.data),

    startTotpSetup: (body: Schemas['ChallengeRequest']) =>
      http
        .post<Schemas['TotpSetupResponse']>(`${BASE}/totp/setup`, body)
        .then((response) => response.data),

    confirmTotp: (body: Schemas['TotpCodeRequest']) =>
      http
        .post<Schemas['AccessTokenResponse']>(`${BASE}/totp/confirm`, body)
        .then((response) => response.data),

    /** Le jeton de rafraîchissement voyage dans le cookie `HttpOnly`. */
    refresh: () =>
      http.post<Schemas['AccessTokenResponse']>(`${BASE}/refresh`).then((response) => response.data),

    logout: () => http.post<void>(`${BASE}/logout`).then(() => undefined),
  }
}
