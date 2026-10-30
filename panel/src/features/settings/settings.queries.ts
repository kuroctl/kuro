// Lecture des paramètres du projet, seulement avec `system.settings.read`.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useSettingsApi } from './settings.api'

export function usePlatformSettings(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['settings'],
      queryFn: () => useSettingsApi().get(),
      enabled: toValue(enabled),
    })),
  )
}

export function useInvalidateSettings() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['settings'] })
}
