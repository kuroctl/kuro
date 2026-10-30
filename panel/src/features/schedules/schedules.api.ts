// Appels API des tâches planifiées d'un serveur (ADR-0022). Aucune logique ici : les erreurs
// remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const base = (server: string) => `/admin/v1/servers/${server}/schedules`

export type Schedule = Schemas['ScheduleDto']
export type ScheduleRequest = Schemas['ScheduleRequest']
export type ScheduleStep = Schemas['StepDto']
export type ScheduleRun = Schemas['RunDto']

export function useSchedulesApi() {
  return {
    list: (server: string) => http.get<Schedule[]>(base(server)).then((response) => response.data),

    get: (server: string, id: string) => http.get<Schedule>(`${base(server)}/${id}`).then((response) => response.data),

    create: (server: string, body: ScheduleRequest) => http.post<Schedule>(base(server), body).then((response) => response.data),

    update: (server: string, id: string, body: ScheduleRequest) =>
      http.put<Schedule>(`${base(server)}/${id}`, body).then((response) => response.data),

    remove: (server: string, id: string) => http.delete<void>(`${base(server)}/${id}`).then(() => undefined),

    run: (server: string, id: string) => http.post<ScheduleRun>(`${base(server)}/${id}/run`).then((response) => response.data),

    runs: (server: string, id: string) => http.get<ScheduleRun[]>(`${base(server)}/${id}/runs`).then((response) => response.data),

    preview: (server: string, body: Schemas['PreviewRequest']) =>
      http.post<Schemas['PreviewDto']>(`${base(server)}/preview`, body).then((response) => response.data),
  }
}
