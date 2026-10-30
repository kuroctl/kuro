// Appels API des serveurs. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/servers'

export type Server = Schemas['ServerDto']
export type PowerAction = Schemas['PowerActionDto']
export type ReinstallMode = Schemas['ReinstallModeDto']
export type ServerPage = Schemas['ServerPageDto']
export type ActionResult = Schemas['ActionResultDto']
export type BulkAction = Schemas['BulkActionDto']
export type BulkOutcome = Schemas['BulkOutcomeDto']
/** Critères de `GET /servers/search` (ADR-0025) : valeurs multiples séparées par des virgules. */
export type ServerSearch = Record<string, string | number | boolean | undefined>

export function useServersApi() {
  return {
    list: () => http.get<Server[]>(BASE).then((response) => response.data),

    search: (params: ServerSearch) => http.get<ServerPage>(`${BASE}/search`, { params }).then((response) => response.data),

    bulk: (action: BulkAction, servers: string[], backup = false) =>
      http.post<BulkOutcome[]>(`${BASE}/bulk`, { action, servers, backup }).then((response) => response.data),

    get: (id: string) => http.get<Server>(`${BASE}/${id}`).then((response) => response.data),

    create: (body: Schemas['CreateServerRequest']) => http.post<Server>(BASE, body).then((response) => response.data),

    updateSettings: (id: string, settings: Record<string, unknown>) =>
      http.patch<Server>(`${BASE}/${id}/settings`, { settings }).then((response) => response.data),

    updateResources: (id: string, body: Schemas['UpdateResourcesRequest']) =>
      http.patch<Server>(`${BASE}/${id}/resources`, body).then((response) => response.data),

    suspend: (id: string) => http.post<Server>(`${BASE}/${id}/suspend`).then((response) => response.data),

    unsuspend: (id: string) => http.post<Server>(`${BASE}/${id}/unsuspend`).then((response) => response.data),

    power: (id: string, action: PowerAction) => http.post<void>(`${BASE}/${id}/power`, { action }).then(() => undefined),

    remove: (id: string) => http.delete<void>(`${BASE}/${id}`).then(() => undefined),

    reinstall: (id: string, mode: ReinstallMode) =>
      http.post<void>(`${BASE}/${id}/reinstall`, { mode }).then(() => undefined),

    consoleTicket: (id: string) =>
      http.post<Schemas['ConsoleTicketDto']>(`${BASE}/${id}/console/ticket`).then((response) => response.data),

    command: (id: string, line: string) => http.post<void>(`${BASE}/${id}/command`, { line }).then(() => undefined),

    runAction: (id: string, key: string, params: Record<string, unknown>) =>
      http.post<ActionResult>(`${BASE}/${id}/actions/${key}`, { params }).then((response) => response.data),
  }
}
