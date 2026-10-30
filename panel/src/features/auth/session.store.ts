// Session admin : jeton d'accès **en mémoire uniquement** (jamais dans le stockage du navigateur).
// Au chargement, la session est reprise grâce au cookie de rafraîchissement `HttpOnly`.
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { meQuery } from '@/features/account/account.queries'
import { configureAuth } from '@/shared/api/http'
import { queryClient } from '@/shared/query/client'

import { useAuthApi } from './auth.api'

type Status = 'unknown' | 'authenticated' | 'anonymous'

const REFRESH_LOCK = 'kuro.session.refresh'

/**
 * Verrou partagé par les onglets (Web Locks) : un seul rafraîchit à la fois, le suivant envoie
 * donc le cookie déjà renouvelé. Sans support du navigateur, chaque onglet reste seul juge.
 */
async function acrossTabs<T>(task: () => Promise<T>): Promise<T> {
  return 'locks' in navigator ? navigator.locks.request(REFRESH_LOCK, task) : task()
}

export const useSessionStore = defineStore('session', () => {
  const accessToken = ref<string | null>(null)
  const status = ref<Status>('unknown')
  const authenticated = computed(() => status.value === 'authenticated')

  function open(token: string) {
    accessToken.value = token
    status.value = 'authenticated'
  }

  function close() {
    accessToken.value = null
    status.value = 'anonymous'
    queryClient.clear()
  }

  // L'API fait tourner le cookie à chaque rafraîchissement : deux appels concurrents (reprise de
  // session et jeton expiré, ou deux onglets) enverraient le même cookie, et l'API, y voyant un
  // vol, révoquerait toute la session.
  let refreshing: Promise<string | null> | null = null

  /** Nouveau jeton à partir du cookie, ou `null` si la session est perdue. Un seul appel à la fois. */
  function refresh(): Promise<string | null> {
    refreshing ??= acrossTabs(() => useAuthApi().refresh())
      .then(({ access_token }) => {
        open(access_token)
        return access_token
      })
      .catch(() => {
        close()
        return null
      })
      .finally(() => {
        refreshing = null
      })
    return refreshing
  }

  /** Premier passage : reprise de session silencieuse, puis chargement du profil. */
  async function restore() {
    if (status.value !== 'unknown') return
    if (await refresh()) {
      await queryClient.prefetchQuery(meQuery)
    }
  }

  return { accessToken, status, authenticated, open, close, refresh, restore }
})

/** Branche la session sur la couche HTTP (jeton, rafraîchissement, session perdue). */
export function connectSessionToHttp(onSessionLost: () => void) {
  const session = useSessionStore()
  configureAuth({
    getAccessToken: () => session.accessToken,
    refresh: session.refresh,
    onSessionLost: () => {
      session.close()
      onSessionLost()
    },
  })
}
