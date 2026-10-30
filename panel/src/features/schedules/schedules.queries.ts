// Lectures des tâches planifiées, rafraîchies souvent tant qu'une exécution tourne.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { FAST_REFRESH_MS, SLOW_REFRESH_MS } from '@/features/servers/state'

import { type Schedule, type ScheduleRun, useSchedulesApi } from './schedules.api'

export function useSchedules(server: MaybeRefOrGetter<string>, enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'schedules'],
      queryFn: () => useSchedulesApi().list(toValue(server)),
      enabled: toValue(enabled),
      refetchInterval: (query: { state: { data?: Schedule[] } }) =>
        query.state.data?.some((schedule) => schedule.running) ? FAST_REFRESH_MS : SLOW_REFRESH_MS,
    })),
  )
}

export function useSchedule(server: MaybeRefOrGetter<string>, id: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'schedules', toValue(id)],
      queryFn: () => useSchedulesApi().get(toValue(server), toValue(id)!),
      enabled: !!toValue(id),
    })),
  )
}

export function useScheduleRuns(server: MaybeRefOrGetter<string>, id: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'schedules', toValue(id), 'runs'],
      queryFn: () => useSchedulesApi().runs(toValue(server), toValue(id)!),
      enabled: !!toValue(id),
      refetchInterval: (query: { state: { data?: ScheduleRun[] } }) =>
        query.state.data?.[0]?.outcome === 'running' ? FAST_REFRESH_MS : SLOW_REFRESH_MS,
    })),
  )
}

export function useInvalidateSchedules() {
  const client = useQueryClient()
  return (server: string) => client.invalidateQueries({ queryKey: ['servers', server, 'schedules'] })
}
