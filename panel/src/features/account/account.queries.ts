// Lectures mises en cache (Vue Query) du compte connecté.
import { queryOptions, useQuery } from '@tanstack/vue-query'
import { computed } from 'vue'

import { useAccountApi } from './account.api'

export const meQuery = queryOptions({
  queryKey: ['me'],
  queryFn: () => useAccountApi().me(),
})

export function useMe() {
  return useQuery(meQuery)
}

/** Droits du compte connecté, pour afficher ou masquer une action. L'API reste seule juge. */
export function usePermissions() {
  const { data } = useMe()
  const can = (permission: string) =>
    Boolean(data.value?.all_permissions || data.value?.permissions.includes(permission))
  return { can, profile: computed(() => data.value) }
}
