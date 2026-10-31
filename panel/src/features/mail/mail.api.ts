// Appels API de l'envoi d'e-mails (ADR-0046) : réglages SMTP du projet et test. Aucune logique
// ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/settings/mail'

export type MailSettings = Schemas['MailSettingsDto']
export type MailSettingsRequest = Schemas['MailSettingsRequest']

export function useMailApi() {
  return {
    get: () => http.get<MailSettings>(BASE).then((response) => response.data),

    /** Le mot de passe est chiffré par l'API et jamais renvoyé ; absent, celui en place est gardé. */
    set: (body: MailSettingsRequest) => http.put<MailSettings>(BASE, body).then((response) => response.data),

    remove: () => http.delete<void>(BASE).then(() => undefined),

    /** E-mail de test à l'adresse de l'admin connecté. */
    test: () => http.post<Schemas['MailTestDto']>(`${BASE}/test`).then((response) => response.data),
  }
}
