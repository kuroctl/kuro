// Lectures du DNS : la zone publique (pour composer les adresses) et la configuration complète,
// seulement avec `system.dns.manage`.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useDnsApi } from './dns.api'

/** Zone publique : `base_domain` absent tant que le DNS n'est ni configuré ni activé. */
export function useDnsZone() {
  return useQuery({
    queryKey: ['dns', 'zone'],
    queryFn: () => useDnsApi().zone(),
    staleTime: 60_000,
  })
}

export function useDnsSettings(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['dns', 'settings'],
      queryFn: () => useDnsApi().settings(),
      enabled: toValue(enabled),
    })),
  )
}

/** Après une écriture : la configuration et la zone publique. */
export function useInvalidateDns() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['dns'] })
}
