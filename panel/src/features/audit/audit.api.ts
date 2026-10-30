// Appels API du journal d'audit (ADR-0008, ADR-0019). Aucune logique ici : les erreurs remontent à
// l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/audit'

export type AuditEntry = Schemas['AuditEntryDto']
export type AuditPage = Schemas['AuditPage']
export type AuditVerify = Schemas['AuditVerifyDto']

/** Filtres de la liste et de l'export. */
export interface AuditFilters {
  action?: string
  actor_kind?: string
  outcome?: string
  /** RFC 3339. */
  from?: string
  to?: string
}

export function useAuditApi() {
  return {
    list: (filters: AuditFilters, before_seq?: number) =>
      http.get<AuditPage>(BASE, { params: { ...filters, before_seq } }).then((response) => response.data),

    verify: () => http.post<AuditVerify>(`${BASE}/verify`).then((response) => response.data),

    exportCsv: (filters: AuditFilters) =>
      http
        .get<Blob>(`${BASE}/export`, { params: filters, responseType: 'blob' })
        .then((response) => ({ blob: response.data, disposition: String(response.headers['content-disposition'] ?? '') })),
  }
}
