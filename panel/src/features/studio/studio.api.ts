// Appels API du Prefab Studio. Aucune logique ici : les erreurs remontent à l'appelant.
import type { PrefabAssetKind } from '@/features/prefabs/prefabs.api'
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/studio'

export type Draft = Schemas['DraftDto']
export type DraftSummary = Schemas['DraftSummaryDto']
export type DraftCheck = Schemas['CheckDto']
export type ComposeConversion = Schemas['ComposeDto']
/** Chemin relatif → contenu texte. */
export type DraftFiles = Record<string, string>

export function useStudioApi() {
  return {
    schema: () => http.get<Record<string, unknown>>(`${BASE}/schema`).then((response) => response.data),

    list: () => http.get<DraftSummary[]>(`${BASE}/drafts`).then((response) => response.data),

    get: (id: string) => http.get<Draft>(`${BASE}/drafts/${id}`).then((response) => response.data),

    create: (
      start: { from?: { id: string; version: string }; compose?: { text: string; id?: string }; egg?: { text: string; id?: string } } = {},
    ) =>
      http
        .post<Draft>(`${BASE}/drafts`, { from: start.from ?? null, compose: start.compose ?? null, egg: start.egg ?? null })
        .then((response) => response.data),

    convertCompose: (text: string, id?: string) =>
      http.post<ComposeConversion>(`${BASE}/compose`, { text, id: id || null }).then((response) => response.data),

    convertEgg: (text: string, id?: string) =>
      http.post<ComposeConversion>(`${BASE}/egg`, { text, id: id || null }).then((response) => response.data),

    save: (id: string, files: DraftFiles) => http.put<Draft>(`${BASE}/drafts/${id}`, { files }).then((response) => response.data),

    remove: (id: string) => http.delete<void>(`${BASE}/drafts/${id}`).then(() => undefined),

    validate: (files: DraftFiles) => http.post<DraftCheck>(`${BASE}/validate`, { files }).then((response) => response.data),

    asset: (id: string, kind: PrefabAssetKind) =>
      http.get<Blob>(`${BASE}/drafts/${id}/assets/${kind}`, { responseType: 'blob' }).then((response) => response.data),

    putAsset: (id: string, kind: PrefabAssetKind, image: Blob) =>
      http
        .put<void>(`${BASE}/drafts/${id}/assets/${kind}`, image, { headers: { 'Content-Type': image.type || 'application/octet-stream' } })
        .then(() => undefined),

    removeAsset: (id: string, kind: PrefabAssetKind) => http.delete<void>(`${BASE}/drafts/${id}/assets/${kind}`).then(() => undefined),

    importSteamAssets: (id: string) =>
      http.post<Schemas['SteamImportDto']>(`${BASE}/drafts/${id}/assets/steam`).then((response) => response.data),

    startTrial: (id: string, body: Schemas['StartTrialRequest']) =>
      http.post<Schemas['TrialStartedDto']>(`${BASE}/drafts/${id}/trial`, body).then((response) => response.data),

    publish: (id: string) => http.post<Schemas['PrefabDto']>(`${BASE}/drafts/${id}/publish`).then((response) => response.data),
  }
}
