// Appels API du contenu des serveurs : mods, plugins (ADR-0020), depuis la source choisie de
// l'emplacement (ADR-0029). Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const base = (server: string) => `/admin/v1/servers/${server}/content`

export type ContentSlot = Schemas['ContentSlotDto']
export type ContentItem = Schemas['ContentDto']
export type ProjectHit = Schemas['ProjectHitDto']
export type ContentVersion = Schemas['ContentVersionDto']
export type ContentLoader = Schemas['ContentLoaderDto']
export type ContentCategory = Schemas['ContentCategoryDto']
export type ContentProject = Schemas['ContentProjectDto']
export type ContentSort = 'relevance' | 'downloads' | 'updated' | 'newest'

/** Filtres de recherche communs aux sources. */
export interface SearchFilters {
  sort: ContentSort
  categories: string[]
}

export function useContentApi() {
  return {
    loaders: () => http.get<ContentLoader[]>('/admin/v1/content/loaders').then((response) => response.data),

    slots: (server: string) => http.get<ContentSlot[]>(base(server)).then((response) => response.data),

    list: (server: string, slot: string) =>
      http.get<Schemas['ContentListDto']>(`${base(server)}/${slot}`).then((response) => response.data),

    search: (server: string, slot: string, source: string, q: string, filters: SearchFilters, offset = 0) =>
      http
        .get<Schemas['ContentSearchDto']>(`${base(server)}/${slot}/search`, {
          params: { q, offset, source, sort: filters.sort, categories: filters.categories.join(',') || undefined },
        })
        .then((response) => response.data),

    categories: (server: string, slot: string, source: string) =>
      http.get<ContentCategory[]>(`${base(server)}/${slot}/categories`, { params: { source } }).then((response) => response.data),

    /** Fiche d'un projet, avec les valeurs que sa description propose pour les champs (ADR-0030). */
    project: (server: string, slot: string, source: string, project: string) =>
      http.get<ContentProject>(`${base(server)}/${slot}/projects/${project}`, { params: { source } }).then((response) => response.data),

    versions: (server: string, slot: string, source: string, project: string) =>
      http
        .get<ContentVersion[]>(`${base(server)}/${slot}/projects/${project}/versions`, { params: { source } })
        .then((response) => response.data),

    install: (server: string, slot: string, source: string, version_id: string, values: Record<string, string | string[]> = {}) =>
      http.post<ContentItem[]>(`${base(server)}/${slot}`, { version_id, source, values }).then((response) => response.data),

    setValues: (server: string, slot: string, id: string, values: Record<string, string | string[]>) =>
      http.put<ContentItem>(`${base(server)}/${slot}/${id}/values`, { values }).then((response) => response.data),

    update: (server: string, slot: string, id: string) =>
      http.post<ContentItem>(`${base(server)}/${slot}/${id}/update`).then((response) => response.data),

    toggle: (server: string, slot: string, id: string, enabled: boolean) =>
      http.patch<ContentItem>(`${base(server)}/${slot}/${id}`, { enabled }).then((response) => response.data),

    upload: (server: string, slot: string, file: Blob, file_name: string, progress?: (ratio: number) => void) =>
      http
        .post<ContentItem>(`${base(server)}/${slot}/upload`, file, {
          params: { file_name },
          headers: { 'content-type': 'application/octet-stream' },
          onUploadProgress: (event) => progress?.(event.total ? event.loaded / event.total : 0),
        })
        .then((response) => response.data),

    modpackSearch: (server: string, slot: string, q: string) =>
      http
        .get<Schemas['ContentSearchDto']>(`${base(server)}/${slot}/modpacks/search`, { params: { q } })
        .then((response) => response.data),

    modpackVersions: (server: string, slot: string, project: string) =>
      http.get<ContentVersion[]>(`${base(server)}/${slot}/modpacks/${project}/versions`).then((response) => response.data),

    installModpack: (server: string, slot: string, version_id: string) =>
      http.post<ContentItem[]>(`${base(server)}/${slot}/modpacks`, { version_id }).then((response) => response.data),

    remove: (server: string, slot: string, id: string, with_orphans: boolean) =>
      http
        .delete<Schemas['RemovedContentDto']>(`${base(server)}/${slot}/${id}`, { params: { with_orphans } })
        .then((response) => response.data),
  }
}
