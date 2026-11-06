// Appels API des tâches de fond (ADR-0062) : liste, simulation, lancement.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1'

export type TaskStatus = Schemas['TaskStatus']

export function useTasksApi() {
  return {
    list: () => http.get<TaskStatus[]>(`${BASE}/settings/tasks`).then((response) => response.data),

    setDryRun: (name: string, dryRun: boolean) =>
      http
        .put<TaskStatus>(`${BASE}/settings/tasks/${name}`, { dry_run: dryRun } satisfies Schemas['TaskUpdateRequest'])
        .then((response) => response.data),

    run: (name: string) => http.post<void>(`${BASE}/settings/tasks/${name}/run`).then(() => undefined),
  }
}
