// Journal d'audit page par page (du plus récent au plus ancien), selon les filtres.
import { useInfiniteQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { type AuditFilters, useAuditApi } from './audit.api'

export function useAuditLog(filters: MaybeRefOrGetter<AuditFilters>) {
  return useInfiniteQuery(
    computed(() => ({
      queryKey: ['audit', toValue(filters)],
      queryFn: ({ pageParam }: { pageParam: number | undefined }) => useAuditApi().list(toValue(filters), pageParam),
      initialPageParam: undefined as number | undefined,
      getNextPageParam: (page: { next_before_seq?: number | null }) => page.next_before_seq ?? undefined,
    })),
  )
}
