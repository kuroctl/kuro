// Lectures des sauvegardes d'un serveur, rafraîchies souvent tant qu'une sauvegarde est en cours.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { FAST_REFRESH_MS, SLOW_REFRESH_MS } from '@/features/servers/state'

import { useBackupsApi } from './backups.api'

export function useBackups(server: MaybeRefOrGetter<string>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'backups'],
      queryFn: () => useBackupsApi().list(toValue(server)),
      refetchInterval: (query: { state: { data?: { backups: { state: string }[] } } }) =>
        query.state.data?.backups.some((backup) => backup.state === 'creating') ? FAST_REFRESH_MS : SLOW_REFRESH_MS,
    })),
  )
}

/** Après une écriture : les sauvegardes et l'état du serveur se rechargent. */
export function useInvalidateBackups() {
  const client = useQueryClient()
  return (server: string) =>
    Promise.all([
      client.invalidateQueries({ queryKey: ['servers', server, 'backups'] }),
      client.invalidateQueries({ queryKey: ['servers', server], exact: true }),
    ])
}
