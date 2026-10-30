// Lectures des mises à jour de prefab : versions proposées à un serveur, aperçu, serveurs
// plus anciens d'une version.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useUpgradesApi } from './upgrades.api'

/**
 * Versions proposées. `current` (version actuelle du serveur) fait partie de la clé : la mise à
 * jour se termine en arrière-plan, et c'est le passage du serveur à sa nouvelle version, vu par
 * son propre rafraîchissement, qui relance cette lecture.
 */
export function useServerUpgrades(
  server: MaybeRefOrGetter<string>,
  current: MaybeRefOrGetter<string>,
  enabled: MaybeRefOrGetter<boolean> = true,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['upgrades', 'server', toValue(server), toValue(current)],
      queryFn: () => useUpgradesApi().available(toValue(server)),
      enabled: toValue(enabled),
    })),
  )
}

export function useUpgradePreview(server: MaybeRefOrGetter<string>, to: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['upgrades', 'preview', toValue(server), toValue(to)],
      queryFn: () => useUpgradesApi().preview(toValue(server), toValue(to)!),
      enabled: Boolean(toValue(to)),
      retry: false,
    })),
  )
}

export function useUpgradeCandidates(
  prefab: MaybeRefOrGetter<string>,
  version: MaybeRefOrGetter<string | undefined>,
  enabled: MaybeRefOrGetter<boolean> = true,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['upgrades', 'candidates', toValue(prefab), toValue(version)],
      queryFn: () => useUpgradesApi().candidates(toValue(prefab), toValue(version)!),
      enabled: toValue(enabled) && Boolean(toValue(version)),
    })),
  )
}

/** Après une mise à jour : versions proposées, aperçus et serveurs se rechargent. */
export function useInvalidateUpgrades() {
  const client = useQueryClient()
  return () => Promise.all([
    client.invalidateQueries({ queryKey: ['upgrades'] }),
    client.invalidateQueries({ queryKey: ['servers'] }),
  ])
}
