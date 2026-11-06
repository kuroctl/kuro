// Lectures des tâches de fond (ADR-0062), rafraîchies pour suivre un passage lancé.
import { useQuery, useQueryClient } from '@tanstack/vue-query'

import { useTasksApi } from './tasks.api'

export function useTasks() {
  return useQuery({ queryKey: ['tasks'], queryFn: () => useTasksApi().list(), refetchInterval: 5_000 })
}

export function useInvalidateTasks() {
  const client = useQueryClient()
  return () => Promise.all([client.invalidateQueries({ queryKey: ['tasks'] }), client.invalidateQueries({ queryKey: ['machines'] })])
}
