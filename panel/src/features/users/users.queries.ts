// Lectures des comptes admin.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useUsersApi } from './users.api'

export function useAdminUsers(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(computed(() => ({ queryKey: ['users'], queryFn: () => useUsersApi().list(), enabled: toValue(enabled) })))
}

export function useAdminUser(id: MaybeRefOrGetter<string>) {
  return useQuery(computed(() => ({ queryKey: ['users', toValue(id)], queryFn: () => useUsersApi().get(toValue(id)) })))
}

/** Après une écriture : comptes et rôles (nombre de membres) se rechargent. */
export function useInvalidateUsers() {
  const client = useQueryClient()
  return () =>
    Promise.all([client.invalidateQueries({ queryKey: ['users'] }), client.invalidateQueries({ queryKey: ['roles'] })])
}
