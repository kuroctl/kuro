// Lecture des réglages SMTP (ADR-0046) et invalidation après un changement. `configured` dit
// si l'e-mail est proposable (canal, invitation).
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useMailApi } from './mail.api'

export function useMailSettings(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['settings', 'mail'],
      queryFn: () => useMailApi().get(),
      enabled: toValue(enabled),
    })),
  )
}

export function useInvalidateMail() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['settings', 'mail'] })
}
