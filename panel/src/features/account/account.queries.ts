// Lectures mises en cache (Vue Query) du compte connecté.
import { queryOptions, useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed } from 'vue'

import { useAccountApi } from './account.api'

export const meQuery = queryOptions({
  queryKey: ['me'],
  queryFn: () => useAccountApi().me(),
})

export function useMe() {
  return useQuery(meQuery)
}

export function useInvalidateMe() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: meQuery.queryKey })
}

/** Droits du compte connecté, pour afficher ou masquer une action. L'API reste seule juge. */
export function usePermissions() {
  const { data } = useMe()
  const can = (permission: string) =>
    Boolean(data.value?.all_permissions || data.value?.permissions.includes(permission))
  return { can, profile: computed(() => data.value) }
}

export const sshKeysQuery = queryOptions({
  queryKey: ['me', 'ssh-keys'],
  queryFn: () => useAccountApi().sshKeys(),
})

export function useSshKeys() {
  return useQuery(sshKeysQuery)
}

export function useInvalidateSshKeys() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: sshKeysQuery.queryKey })
}
