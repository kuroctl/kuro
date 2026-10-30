// Lectures mises en cache des nœuds et de leurs ports.
import { queryOptions, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useNodesApi } from './nodes.api'

export const nodesQuery = queryOptions({
  queryKey: ['nodes'],
  queryFn: () => useNodesApi().list(),
  refetchInterval: 15000,
})

export function useNodes() {
  return useQuery(nodesQuery)
}

export function useNode(id: MaybeRefOrGetter<string>) {
  return useQuery(computed(() => ({ queryKey: ['nodes', toValue(id)], queryFn: () => useNodesApi().get(toValue(id)) })))
}

export function allocationsQuery(id: string) {
  return queryOptions({
    queryKey: ['nodes', id, 'allocations'],
    queryFn: () => useNodesApi().allocations(id),
  })
}

export function useAllocations(id: MaybeRefOrGetter<string>) {
  return useQuery(computed(() => allocationsQuery(toValue(id))))
}

/** Après une écriture : le nœud, la liste et les régions (elles comptent leurs nœuds). */
export function useInvalidateNodes() {
  const client = useQueryClient()
  return () => Promise.all([client.invalidateQueries({ queryKey: ['nodes'] }), client.invalidateQueries({ queryKey: ['regions'] })])
}
