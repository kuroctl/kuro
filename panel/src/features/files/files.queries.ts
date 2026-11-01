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

/** Versions d'un fichier, relues à chaque ouverture du panneau. */
export function useFileHistory(server: MaybeRefOrGetter<string>, path: MaybeRefOrGetter<string>, enabled: MaybeRefOrGetter<boolean>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'files', 'history', toValue(path)],
      queryFn: () => useFilesApi().history(toValue(server), toValue(path)),
      enabled: toValue(enabled),
      staleTime: 0,
    })),
  )
}

/** Fichiers supprimés d'un dossier (sous un volume seulement). */
export function useDeletedFiles(server: MaybeRefOrGetter<string>, path: MaybeRefOrGetter<string>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'files', 'deleted', toValue(path)],
      queryFn: () => useFilesApi().deleted(toValue(server), toValue(path)),
      enabled: toValue(path).split('/').filter(Boolean).length >= 2,
      retry: false,
    })),
  )
}

/** Contenu d'une version : immuable, gardé tant qu'on le regarde. */
export function useFileVersion(
  server: MaybeRefOrGetter<string>,
  path: MaybeRefOrGetter<string>,
  version: MaybeRefOrGetter<string | undefined>,
) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'files', 'version', toValue(path), toValue(version)],
      queryFn: () => useFilesApi().version(toValue(server), toValue(path), toValue(version) ?? ''),
      enabled: Boolean(toValue(version)),
      staleTime: Infinity,
      retry: false,
    })),
  )
}

/** Coordonnées SFTP, demandées seulement avec `servers.files.sftp`. */
export function useSftpConnection(server: MaybeRefOrGetter<string>, enabled: MaybeRefOrGetter<boolean>) {
  return useQuery(
    computed(() => ({
      queryKey: ['servers', toValue(server), 'sftp'],
      queryFn: () => useFilesApi().sftp(toValue(server)),
      enabled: toValue(enabled),
    })),
  )
}

/** Après une écriture : listes et contenus du serveur se rechargent. */
export function useInvalidateFiles() {
  const client = useQueryClient()
  return (server: string) => client.invalidateQueries({ queryKey: ['servers', server, 'files'] })
}
