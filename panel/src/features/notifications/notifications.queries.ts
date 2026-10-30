// Lectures des notifications (ADR-0044), seulement avec `system.notifications.read`.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useNotificationsApi } from './notifications.api'

export function useNotificationEvents(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['notifications', 'events'],
      queryFn: () => useNotificationsApi().events(),
      staleTime: Infinity,
      enabled: toValue(enabled),
    })),
  )
}

export function useNotificationChannels(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['notifications', 'channels'],
      queryFn: () => useNotificationsApi().channels(),
      enabled: toValue(enabled),
    })),
  )
}

export function useNotificationRules(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['notifications', 'rules'],
      queryFn: () => useNotificationsApi().rules(),
      enabled: toValue(enabled),
    })),
  )
}

/** Journal des envois, rafraîchi toutes les 15 s tant que la page est ouverte. */
export function useNotificationDeliveries(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['notifications', 'deliveries'],
      queryFn: () => useNotificationsApi().deliveries(50),
      refetchInterval: 15_000,
      enabled: toValue(enabled),
    })),
  )
}

export function useServerSubscriptions(server: MaybeRefOrGetter<string>, enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['notifications', 'servers', toValue(server), 'subscriptions'],
      queryFn: () => useNotificationsApi().subscriptions(toValue(server)),
      enabled: toValue(enabled),
    })),
  )
}

export function useServerDeliveries(server: MaybeRefOrGetter<string>, enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['notifications', 'servers', toValue(server), 'deliveries'],
      queryFn: () => useNotificationsApi().serverDeliveries(toValue(server)),
      refetchInterval: 15_000,
      enabled: toValue(enabled),
    })),
  )
}

/** Après une écriture : canaux, règles, abonnements et journaux. */
export function useInvalidateNotifications() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ['notifications'] })
}
