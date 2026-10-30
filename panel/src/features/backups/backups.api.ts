// Appels API des sauvegardes d'un serveur (ADR-0018). Aucune logique ici : les erreurs remontent
// à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const base = (server: string) => `/admin/v1/servers/${server}/backups`

export type Backup = Schemas['BackupDto']
export type BackupList = Schemas['BackupListDto']
export type RestoreMode = Schemas['RestoreModeDto']

export function useBackupsApi() {
  return {
    list: (server: string) => http.get<BackupList>(base(server)).then((response) => response.data),

    create: (server: string, name?: string) =>
      http.post<Backup>(base(server), name ? { name } : {}).then((response) => response.data),

    setLocked: (server: string, backup: string, locked: boolean) =>
      http.patch<Backup>(`${base(server)}/${backup}`, { locked }).then((response) => response.data),

    restore: (server: string, backup: string, mode: RestoreMode) =>
      http.post<void>(`${base(server)}/${backup}/restore`, { mode }).then(() => undefined),

    remove: (server: string, backup: string) => http.delete<void>(`${base(server)}/${backup}`).then(() => undefined),

    downloadTicket: (server: string, backup: string) =>
      http.post<Schemas['DownloadTicketDto']>(`${base(server)}/${backup}/download/ticket`).then((response) => response.data),
  }
}
