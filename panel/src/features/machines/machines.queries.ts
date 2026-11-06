// Lectures des machines louées (ADR-0065) et de leurs réglages.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { regionsQuery } from '@/features/regions/regions.queries'

import { useMachinesApi } from './machines.api'

/** Machines, rafraîchies tant qu'une opération longue est en cours (création, sommeil, réveil). */
export function useMachines(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['machines'],
      queryFn: () => useMachinesApi().list(),
      enabled: toValue(enabled),
      refetchInterval: 10_000,
    })),
  )
}

export function useMachinesSettings(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['machines', 'settings'],
      queryFn: () => useMachinesApi().settings(),
      enabled: toValue(enabled),
    })),
  )
}

/** Emplacements de chaque fournisseur configuré, par fournisseur (liste des régions louées). */
export function useProviderLocations(providers: MaybeRefOrGetter<string[]>) {
  return useQuery(
    computed(() => ({
      queryKey: ['machines', 'locations', toValue(providers)],
      queryFn: async () => {
        const names = toValue(providers)
        const lists = await Promise.all(names.map((name) => useMachinesApi().locations(name)))
        return Object.fromEntries(names.map((name, i) => [name, lists[i] ?? []]))
      },
      enabled: toValue(providers).length > 0,
      staleTime: 300_000,
    })),
  )
}

export function useMachineOffers(regionId: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['machines', 'offers', toValue(regionId)],
      queryFn: () => useMachinesApi().offers(toValue(regionId) as string),
      enabled: !!toValue(regionId),
      staleTime: 60_000,
    })),
  )
}

/** Après une écriture : machines, réglages, et les nœuds qu'elles portent. */
export function useInvalidateMachines() {
  const client = useQueryClient()
  return () => Promise.all([client.invalidateQueries({ queryKey: ['machines'] }), client.invalidateQueries({ queryKey: ['nodes'] })])
}

/**
 * Régions où louer : celles qui ont un fournisseur (lues dans les réglages avec
 * `system.settings.read`) ; sans cette permission, toutes, et l'API refuse les autres.
 */
export function useRentableRegions(canReadSettings: MaybeRefOrGetter<boolean>) {
  const settings = useMachinesSettings(canReadSettings)
  const regions = useQuery(regionsQuery)
  return computed(() => {
    if (toValue(canReadSettings)) {
      return (settings.data.value?.regions ?? []).filter((r) => r.provider).map((r) => ({ id: r.region_id, name: r.name }))
    }
    return (regions.data.value ?? []).map((r) => ({ id: r.id, name: r.name }))
  })
}
