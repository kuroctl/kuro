// Lectures du contenu d'un serveur, rafraîchies souvent tant qu'une installation est en cours.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { FAST_REFRESH_MS, SLOW_REFRESH_MS } from '@/features/servers/state'

import { type SearchFilters, useContentApi } from './content.api'

/** Loaders de la source et leurs icônes : changent rarement. */
export function useContentLoaders(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['content', 'loaders'],
      queryFn: () => useContentApi().loaders(),
      enabled: toValue(enabled),
      staleTime: 60 * 60 * 1000,
    })),
  )
}

export function useContentSlots(server: MaybeRefOrGetter<string>, enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'content'],
      queryFn: () => useContentApi().slots(toValue(server)),
      enabled: toValue(enabled) && !!toValue(server),
    })),
  )
}

export function useInstalledContent(server: MaybeRefOrGetter<string>, slot: MaybeRefOrGetter<string>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'content', toValue(slot)],
      queryFn: () => useContentApi().list(toValue(server), toValue(slot)),
      refetchInterval: (query: { state: { data?: { items: { state: string }[] } } }) =>
        query.state.data?.items.some((item) => item.state === 'installing') ? FAST_REFRESH_MS : SLOW_REFRESH_MS,
    })),
  )
}

export function useContentSearch(
  server: MaybeRefOrGetter<string>,
  slot: MaybeRefOrGetter<string>,
  source: MaybeRefOrGetter<string>,
  text: MaybeRefOrGetter<string>,
  filters: MaybeRefOrGetter<SearchFilters>,
  enabled: MaybeRefOrGetter<boolean>,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'content', toValue(slot), 'search', toValue(source), toValue(text), toValue(filters)],
      queryFn: () => useContentApi().search(toValue(server), toValue(slot), toValue(source), toValue(text), toValue(filters)),
      enabled: toValue(enabled),
      // La liste précédente reste affichée pendant la frappe.
      placeholderData: <T>(previous: T) => previous,
      staleTime: 60_000,
    })),
  )
}

/** Catégories d'une source pour l'emplacement : changent rarement. */
export function useContentCategories(
  server: MaybeRefOrGetter<string>,
  slot: MaybeRefOrGetter<string>,
  source: MaybeRefOrGetter<string>,
  enabled: MaybeRefOrGetter<boolean>,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'content', toValue(slot), 'categories', toValue(source)],
      queryFn: () => useContentApi().categories(toValue(server), toValue(slot), toValue(source)),
      enabled: toValue(enabled),
      staleTime: 60 * 60 * 1000,
    })),
  )
}

export function useContentVersions(
  server: MaybeRefOrGetter<string>,
  slot: MaybeRefOrGetter<string>,
  source: MaybeRefOrGetter<string>,
  project: MaybeRefOrGetter<string | undefined>,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'content', toValue(slot), 'versions', toValue(source), toValue(project)],
      queryFn: () => useContentApi().versions(toValue(server), toValue(slot), toValue(source), toValue(project)!),
      enabled: !!toValue(project),
      staleTime: 60_000,
    })),
  )
}

/** Fiche d'un projet, demandée seulement si l'emplacement a des valeurs à pré-remplir. */
export function useContentProject(
  server: MaybeRefOrGetter<string>,
  slot: MaybeRefOrGetter<string>,
  source: MaybeRefOrGetter<string>,
  project: MaybeRefOrGetter<string | undefined>,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'content', toValue(slot), 'project', toValue(source), toValue(project)],
      queryFn: () => useContentApi().project(toValue(server), toValue(slot), toValue(source), toValue(project)!),
      enabled: !!toValue(project),
      staleTime: 60_000,
    })),
  )
}

export function useInvalidateContent() {
  const client = useQueryClient()
  return (server: string) => client.invalidateQueries({ queryKey: ['servers', server, 'content'] })
}
