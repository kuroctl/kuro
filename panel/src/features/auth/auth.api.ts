// Appels API de l'authentification admin. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/auth'

export function useAuthApi() {
  return {
    login: (body: Schemas['LoginRequest']) =>
      http.post<Schemas['LoginResponse']>(`${BASE}/login`, body).then((response) => response.data),

    /** Invitation acceptée (ADR-0046) : mot de passe choisi, puis même réponse que `login`. */
    acceptInvitation: (body: Schemas['InvitationRequest']) =>
      http.post<Schemas['LoginResponse']>(`${BASE}/invitation`, body).then((response) => response.data),

    /** Mot de passe oublié (ADR-0049) : même réponse que le compte existe ou non. */
    forgotPassword: (body: Schemas['ForgotPasswordRequest']) =>
      http.post<void>(`${BASE}/password/forgot`, body).then(() => undefined),

    /** Lien « mot de passe oublié » utilisé : nouveau mot de passe, puis même réponse que `login`. */
    resetPassword: (body: Schemas['PasswordResetLinkRequest']) =>
      http.post<Schemas['LoginResponse']>(`${BASE}/password/reset`, body).then((response) => response.data),

    /** Mot de passe choisi par un autre admin, remplacé avant le TOTP : même réponse que `login`. */
    changeRequiredPassword: (body: Schemas['RequiredPasswordRequest']) =>
      http.post<Schemas['LoginResponse']>(`${BASE}/password`, body).then((response) => response.data),

    /** `code` : code TOTP ou code de secours. */
    verifyTotp: (body: Schemas['TotpCodeRequest']) =>
      http
        .post<Schemas['AccessTokenResponse']>(`${BASE}/totp/verify`, body)
        .then((response) => response.data),

    startTotpSetup: (body: Schemas['ChallengeRequest']) =>
      http
        .post<Schemas['TotpSetupResponse']>(`${BASE}/totp/setup`, body)
        .then((response) => response.data),

    /** Session ouverte et codes de secours, montrés cette seule fois. */
    confirmTotp: (body: Schemas['TotpCodeRequest']) =>
      http
        .post<Schemas['TotpConfirmedResponse']>(`${BASE}/totp/confirm`, body)
        .then((response) => response.data),

    /** Le jeton de rafraîchissement voyage dans le cookie `HttpOnly`. */
    refresh: () =>
      http.post<Schemas['AccessTokenResponse']>(`${BASE}/refresh`).then((response) => response.data),

    logout: () => http.post<void>(`${BASE}/logout`).then(() => undefined),
  }
}
