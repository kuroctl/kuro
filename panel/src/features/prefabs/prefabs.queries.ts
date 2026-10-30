// Lectures mises en cache (Vue Query) du catalogue de prefabs. Un prefab est immuable
// (`id` + `version`) : son manifest et ses traductions ne se périment pas.
import { queryOptions, useQuery, useQueryClient } from '@tanstack/vue-query'
import { useObjectUrl } from '@vueuse/core'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { usePermissions } from '@/features/account/account.queries'

import { type PrefabAssetKind, usePrefabsApi } from './prefabs.api'

export const prefabsQuery = queryOptions({
  queryKey: ['prefabs'],
  queryFn: () => usePrefabsApi().list(),
})

export function usePrefabs(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(computed(() => ({ ...prefabsQuery, enabled: toValue(enabled) })))
}

interface PrefabRef {
  id: string
  version: string
}

/**
 * Prefab du catalogue (nom, images disponibles), sans erreur si l'admin ne peut pas lire les
 * prefabs : il est alors simplement absent.
 */
export function useCatalogEntry(prefab: MaybeRefOrGetter<PrefabRef | undefined>) {
  const { can } = usePermissions()
  const { data: prefabs } = usePrefabs(() => can('prefabs.read'))
  return computed(() => {
    const ref = toValue(prefab)
    return ref && prefabs.value?.find((entry) => entry.id === ref.id && entry.version === ref.version)
  })
}

/**
 * URL locale (`blob:`) d'une image du prefab, si le catalogue la déclare. Téléchargée avec le
 * jeton : une balise `<img>` ne peut pas l'envoyer. Immuable, comme le prefab.
 */
export function usePrefabAsset(prefab: MaybeRefOrGetter<PrefabRef | undefined>, kind: PrefabAssetKind) {
  const entry = useCatalogEntry(prefab)
  const { data } = useQuery(
    computed(() => ({
      queryKey: ['prefabs', entry.value?.id, entry.value?.version, 'assets', kind],
      queryFn: () => usePrefabsApi().asset(entry.value!.id, entry.value!.version, kind),
      enabled: Boolean(entry.value?.assets.includes(kind)),
      staleTime: Infinity,
    })),
  )
  return useObjectUrl(data)
}

export function prefabQuery(id: string, version: string) {
  return queryOptions({
    queryKey: ['prefabs', id, version],
    queryFn: () => usePrefabsApi().get(id, version),
    staleTime: Infinity,
  })
}

export function usePrefab(id: MaybeRefOrGetter<string | undefined>, version: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      ...prefabQuery(toValue(id) ?? '', toValue(version) ?? ''),
      enabled: Boolean(toValue(id) && toValue(version)),
    })),
  )
}

export function usePrefabServers(id: MaybeRefOrGetter<string | undefined>, version: MaybeRefOrGetter<string | undefined>, enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['prefabs', toValue(id), toValue(version), 'servers'],
      queryFn: () => usePrefabsApi().servers(toValue(id)!, toValue(version)!),
      enabled: Boolean(toValue(id) && toValue(version) && toValue(enabled)),
    })),
  )
}

/** Après un import ou une suppression : le catalogue se recharge (les versions restent en cache). */
export function useInvalidatePrefabs() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['prefabs'], exact: true })
}

/** Images d'une version, à relire après leur optimisation. */
export function useInvalidatePrefabAssets() {
  const client = useQueryClient()
  return (id: string, version: string) => client.invalidateQueries({ queryKey: ['prefabs', id, version, 'assets'] })
}
