// Lecture du compte Steam de l'installation et invalidation après un changement : le catalogue
// des prefabs en dépend (un prefab qui exige le compte n'est déployable qu'avec lui).
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useSteamApi } from './steam.api'

export function useSteamAccount(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['settings', 'steam-account'],
      queryFn: () => useSteamApi().get(),
      enabled: toValue(enabled),
    })),
  )
}

export function useInvalidateSteamAccount() {
  const client = useQueryClient()
  return () =>
    Promise.all([
      client.invalidateQueries({ queryKey: ['settings', 'steam-account'] }),
      client.invalidateQueries({ queryKey: ['prefabs'] }),
    ])
}
