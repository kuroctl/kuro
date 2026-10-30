// Appels API du catalogue de prefabs. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

import type { Manifest } from './manifest'

const BASE = '/admin/v1/prefabs'

/** Le manifest est typé côté panel : l'OpenAPI le décrit comme un objet libre. */
export type Prefab = Omit<Schemas['PrefabDto'], 'manifest'> & { manifest: Manifest }

/** Image d'un prefab : icône carrée, bannière ou vignette portrait (jaquette). */
export type PrefabAssetKind = 'icon' | 'cover' | 'thumbnail'

/** Traductions d'un prefab pour une langue (`prefab`, `groups`, `fields`, `files`, `content`). */
export type PrefabMessages = Record<string, unknown>

export function usePrefabsApi() {
  return {
    list: () => http.get<Schemas['PrefabSummaryDto'][]>(BASE).then((response) => response.data),

    /** Un résultat par prefab, avec ses versions ; `q` porte sur l'id, les noms et les étiquettes. */
    search: (params: { q?: string; page?: number; per_page?: number }) =>
      http.get<Schemas['PrefabGroupPageDto']>(`${BASE}/search`, { params }).then((response) => response.data),

    get: (id: string, version: string) =>
      http.get<Prefab>(`${BASE}/${id}/${version}`).then((response) => response.data),

    locale: (id: string, version: string, lang: string) =>
      http.get<PrefabMessages>(`${BASE}/${id}/${version}/locales/${lang}`).then((response) => response.data),

    importArchive: (archive: Blob, onProgress?: (ratio: number) => void) =>
      http
        .post<Prefab>(`${BASE}/import`, archive, {
          headers: { 'Content-Type': 'application/zip' },
          onUploadProgress: (event) => event.total && onProgress?.(event.loaded / event.total),
        })
        .then((response) => response.data),

    reload: () => http.post<Schemas['ReloadReportDto']>(`${BASE}/reload`).then((response) => response.data),

    remove: (id: string, version: string) => http.delete<void>(`${BASE}/${id}/${version}`).then(() => undefined),

    servers: (id: string, version: string) =>
      http.get<Schemas['PrefabServerDto'][]>(`${BASE}/${id}/${version}/servers`).then((response) => response.data),

    optimizeAssets: (id: string, version: string) =>
      http.post<Schemas['OptimizeReportDto']>(`${BASE}/${id}/${version}/assets/optimize`).then((response) => response.data),

    asset: (id: string, version: string, kind: PrefabAssetKind) =>
      http.get<Blob>(`${BASE}/${id}/${version}/assets/${kind}`, { responseType: 'blob' }).then((response) => response.data),

    /** Archive `.kprefab` de la version (ADR-0042). */
    exportArchive: (id: string, version: string) =>
      http.get<Blob>(`${BASE}/${id}/${version}/export`, { responseType: 'blob' }).then((response) => response.data),
  }
}
