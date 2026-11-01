// Appels API du compte connecté. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

export type SshKey = Schemas['SshKeyDto']

export function useAccountApi() {
  return {
    me: () => http.get<Schemas['ProfileDto']>('/admin/v1/me').then((response) => response.data),

    changePassword: (body: Schemas['ChangePasswordRequest']) =>
      http.put<void>('/admin/v1/me/password', body).then(() => undefined),

    /** Élève la session pour les actions protégées (ADR-0050). */
    elevate: (body: Schemas['ElevateRequest']) =>
      http.post<Schemas['ElevationResponse']>('/admin/v1/me/elevate', body).then((response) => response.data),

    /** Premier TOTP, ou remplacement (code actuel ou code de secours exigé). */
    startTotpSetup: (body: Schemas['TotpReauthRequest']) =>
      http.post<Schemas['TotpSetupResponse']>('/admin/v1/me/totp/setup', body).then((response) => response.data),

    confirmTotp: (body: Schemas['TotpConfirmRequest']) =>
      http
        .post<Schemas['RecoveryCodesResponse']>('/admin/v1/me/totp/confirm', body)
        .then((response) => response.data),

    regenerateRecoveryCodes: (body: Schemas['TotpReauthRequest']) =>
      http
        .post<Schemas['RecoveryCodesResponse']>('/admin/v1/me/totp/recovery-codes', body)
        .then((response) => response.data),

    disableTotp: (body: Schemas['TotpReauthRequest']) =>
      http.delete<void>('/admin/v1/me/totp', { data: body }).then(() => undefined),

    /** Clés SSH du compte, pour le SFTP (ADR-0047). */
    sshKeys: () => http.get<SshKey[]>('/admin/v1/me/ssh-keys').then((response) => response.data),

    addSshKey: (body: Schemas['AddSshKeyRequest']) =>
      http.post<SshKey>('/admin/v1/me/ssh-keys', body).then((response) => response.data),

    removeSshKey: (id: string) => http.delete<void>(`/admin/v1/me/ssh-keys/${id}`).then(() => undefined),
  }
}
