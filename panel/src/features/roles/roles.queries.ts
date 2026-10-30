// Lectures des rôles et du catalogue des permissions.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { usePermissions } from '@/features/account/account.queries'

import { type Role, useRolesApi } from './roles.api'

export function useRoles(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(computed(() => ({ queryKey: ['roles'], queryFn: () => useRolesApi().list(), enabled: toValue(enabled) })))
}

export function useRole(id: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['roles', toValue(id)],
      queryFn: () => useRolesApi().get(toValue(id)!),
      enabled: !!toValue(id),
    })),
  )
}

export function usePermissionCatalog() {
  return useQuery({ queryKey: ['permissions'], queryFn: () => useRolesApi().catalog(), staleTime: Infinity })
}

export function useInvalidateRoles() {
  const client = useQueryClient()
  return () => Promise.all([client.invalidateQueries({ queryKey: ['roles'] }), client.invalidateQueries({ queryKey: ['users'] })])
}

/** Ce que le compte connecté peut accorder (anti-escalade, l'API reste seule juge). */
export function useGrantable() {
  const { profile } = usePermissions()
  const holds = (permission: string) =>
    Boolean(profile.value?.all_permissions || profile.value?.permissions.includes(permission))
  const canGrant = (role: Pick<Role, 'permissions'>) =>
    role.permissions.includes('*') ? Boolean(profile.value?.all_permissions) : role.permissions.every(holds)
  return { holds, canGrant }
}
