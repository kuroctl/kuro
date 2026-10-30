// Lectures des fichiers d'un serveur. Le contenu d'un fichier ouvert n'est jamais rechargé
// derrière l'éditeur : seul un enregistrement ou un rechargement explicite le remplace.
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'

import { useFilesApi } from './files.api'

export function useFileListing(server: MaybeRefOrGetter<string>, path: MaybeRefOrGetter<string>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'files', 'list', toValue(path)],
      queryFn: () => useFilesApi().list(toValue(server), toValue(path)),
    })),
  )
}

export function useFileContent(server: MaybeRefOrGetter<string>, path: MaybeRefOrGetter<string | undefined>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'files', 'content', toValue(path)],
      queryFn: () => useFilesApi().read(toValue(server), toValue(path) ?? ''),
      enabled: Boolean(toValue(path)),
      staleTime: Infinity,
      // Fichier refermé : rouvert, il est relu.
      gcTime: 0,
      refetchOnWindowFocus: false,
      retry: false,
    })),
  )
}

/** Après une écriture : listes et contenus du serveur se rechargent. */
export function useInvalidateFiles() {
  const client = useQueryClient()
  return (server: string) => client.invalidateQueries({ queryKey: ['servers', server, 'files'] })
}
