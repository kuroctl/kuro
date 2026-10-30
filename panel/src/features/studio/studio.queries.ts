// Lectures du Prefab Studio. Un brouillon ouvert n'est jamais rechargé derrière l'éditeur.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import type { PrefabAssetKind } from '@/features/prefabs/prefabs.api'

import { useStudioApi } from './studio.api'

export function useDrafts(enabled: MaybeRefOrGetter<boolean> = true) {
  return useQuery(computed(() => ({ queryKey: ['studio', 'drafts'], queryFn: () => useStudioApi().list(), enabled: toValue(enabled) })))
}

export function useDraft(id: MaybeRefOrGetter<string>) {
  return useQuery(
    computed(() => ({
      queryKey: ['studio', 'draft', toValue(id)],
      queryFn: () => useStudioApi().get(toValue(id)),
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      retry: false,
    })),
  )
}

/** JSON Schema du format v1, pour l'autocomplétion. */
export function useStudioSchema() {
  return useQuery({ queryKey: ['studio', 'schema'], queryFn: () => useStudioApi().schema(), staleTime: Infinity })
}

export function useDraftAsset(id: MaybeRefOrGetter<string>, kind: PrefabAssetKind, present: MaybeRefOrGetter<boolean>) {
  return useQuery(
    computed(() => ({
      queryKey: ['studio', 'draft', toValue(id), 'assets', kind],
      queryFn: () => useStudioApi().asset(toValue(id), kind),
      enabled: toValue(present),
    })),
  )
}

export function useInvalidateStudio() {
  const client = useQueryClient()
  return {
    drafts: () => client.invalidateQueries({ queryKey: ['studio', 'drafts'] }),
    /** Brouillon relu (essai en cours) ; l'éditeur garde ses fichiers. */
    draft: (id: string) => client.invalidateQueries({ queryKey: ['studio', 'draft', id], exact: true }),
    assets: (id: string) => client.invalidateQueries({ queryKey: ['studio', 'draft', id, 'assets'] }),
    /** Brouillon enregistré : la réponse de l'API devient la version en cache. */
    store: (draft: { id: string }) => client.setQueryData(['studio', 'draft', draft.id], draft),
  }
}
