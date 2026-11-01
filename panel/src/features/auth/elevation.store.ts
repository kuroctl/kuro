// Élévation de session (ADR-0050) : une seule fenêtre de confirmation à la fois, partagée par
// toutes les requêtes refusées en même temps (`403 elevation_required`).
import { defineStore } from 'pinia'
import { ref } from 'vue'

import { configureElevation, type ElevationMethod } from '@/shared/api/http'

export const useElevationStore = defineStore('elevation', () => {
  /** Preuve demandée ; `null` : fenêtre fermée. */
  const method = ref<ElevationMethod | null>(null)
  let pending: Promise<boolean> | null = null
  let resolve: ((elevated: boolean) => void) | null = null

  function request(next: ElevationMethod): Promise<boolean> {
    if (pending) return pending
    method.value = next
    pending = new Promise((done) => (resolve = done))
    return pending
  }

  /** Fin de la confirmation : session élevée, ou abandon. */
  function settle(elevated: boolean) {
    resolve?.(elevated)
    pending = null
    resolve = null
    method.value = null
  }

  return { method, request, settle }
})

/** Branche la fenêtre de confirmation sur la couche HTTP. */
export function connectElevationToHttp() {
  const store = useElevationStore()
  configureElevation(store.request)
}
