// Lectures des agents IA : demande en attente de consentement, accès accordés.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useOAuthApi } from './oauth.api'

export function useConsentRequest(id: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['oauth', 'request', toValue(id)],
      queryFn: () => useOAuthApi().request(toValue(id)!),
      enabled: Boolean(toValue(id)),
      retry: false,
    })),
  )
}

export function useGrants() {
  return useQuery({ queryKey: ['oauth', 'grants'], queryFn: () => useOAuthApi().grants(), retry: false })
}

export function useInvalidateGrants() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['oauth', 'grants'] })
}
