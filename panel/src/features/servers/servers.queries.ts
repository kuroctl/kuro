// Lectures des serveurs, rafraîchies plus souvent pendant une transition (déploiement,
// démarrage…). Sur la page Console, l'état arrive aussi en direct par le WebSocket.
import { keepPreviousData, queryOptions, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { type ServerPage, type ServerSearch, useServersApi } from './servers.api'
import { FAST_REFRESH_MS, isTransitional, SLOW_REFRESH_MS } from './state'

export const serversQuery = queryOptions({
  queryKey: ['servers'],
  queryFn: () => useServersApi().list(),
  refetchInterval: (query) =>
    query.state.data?.some((server) => isTransitional(server.state)) ? FAST_REFRESH_MS : SLOW_REFRESH_MS,
})

export function useServers() {
  return useQuery(serversQuery)
}

export function serverQuery(id: string) {
  return queryOptions({
    queryKey: ['servers', id],
    queryFn: () => useServersApi().get(id),
    refetchInterval: (query) => (query.state.data && isTransitional(query.state.data.state) ? FAST_REFRESH_MS : SLOW_REFRESH_MS),
  })
}

export function useServer(id: MaybeRefOrGetter<string>) {
  return useQuery(computed(() => serverQuery(toValue(id))))
}

/** Page de la liste (ADR-0025) ; la page précédente reste affichée pendant le chargement. */
export function useServerSearch(params: MaybeRefOrGetter<ServerSearch>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', 'search', toValue(params)],
      queryFn: () => useServersApi().search(toValue(params)),
      placeholderData: keepPreviousData,
      refetchInterval: (query: { state: { data?: ServerPage } }) =>
        query.state.data?.items.some((server) => isTransitional(server.state)) ? FAST_REFRESH_MS : SLOW_REFRESH_MS,
    })),
  )
}

/** Après une écriture : la liste et le détail se rechargent. */
export function useInvalidateServers() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['servers'] })
}
