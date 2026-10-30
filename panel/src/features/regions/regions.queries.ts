// Lectures mises en cache des régions.
import { queryOptions, useQuery, useQueryClient } from '@tanstack/vue-query'

import { useRegionsApi } from './regions.api'

export const regionsQuery = queryOptions({
  queryKey: ['regions'],
  queryFn: () => useRegionsApi().list(),
})

export function useRegions() {
  return useQuery(regionsQuery)
}

/** Après une écriture : régions, et nœuds (leur région et leur position en dépendent). */
export function useInvalidateRegions() {
  const client = useQueryClient()
  return () => Promise.all([client.invalidateQueries({ queryKey: ['regions'] }), client.invalidateQueries({ queryKey: ['nodes'] })])
}
