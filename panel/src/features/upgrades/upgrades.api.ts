// Appels API de la mise à jour d'un serveur vers une version plus récente de son prefab
// (ADR-0024). Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

export type Upgrades = Schemas['UpgradesDto']
export type UpgradePreview = Schemas['UpgradePreviewDto']
export type UpgradeBlocker = Schemas['UpgradeBlockerDto']
export type UpgradeCandidate = Schemas['UpgradeCandidateDto']
export type BulkUpgradeResult = Schemas['BulkUpgradeResultDto']

export function useUpgradesApi() {
  return {
    available: (server: string) =>
      http.get<Upgrades>(`/admin/v1/servers/${server}/upgrades`).then((response) => response.data),

    preview: (server: string, to: string) =>
      http.get<UpgradePreview>(`/admin/v1/servers/${server}/upgrade`, { params: { to } }).then((response) => response.data),

    start: (server: string, to: string, backup: boolean) =>
      http.post<UpgradePreview>(`/admin/v1/servers/${server}/upgrade`, { to, backup }).then((response) => response.data),

    candidates: (prefab: string, version: string) =>
      http.get<UpgradeCandidate[]>(`/admin/v1/prefabs/${prefab}/${version}/upgrades`).then((response) => response.data),

    startMany: (servers: string[], to: string, backup: boolean) =>
      http.post<BulkUpgradeResult[]>('/admin/v1/servers/upgrade', { servers, to, backup }).then((response) => response.data),
  }
}
