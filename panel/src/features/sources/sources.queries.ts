// Lecture des sources de contenu à clé d'API et invalidation après un changement de clé : les
// emplacements de contenu des serveurs en dépendent aussi.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useSourcesApi } from './sources.api'

export function useContentSources(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['settings', 'content-sources'],
      queryFn: () => useSourcesApi().list(),
      enabled: toValue(enabled),
    })),
  )
}

export function useInvalidateSources() {
  const client = useQueryClient()
  return () =>
    Promise.all([
      client.invalidateQueries({ queryKey: ['settings', 'content-sources'] }),
      // Une source qui gagne ou perd sa clé change les emplacements de tous les serveurs.
      client.invalidateQueries({ queryKey: ['servers'], predicate: (query) => query.queryKey.includes('content') }),
    ])
}
