// Appels API du DNS (ADR-0026) : zone publique, configuration du fournisseur, nom d'un serveur,
// adresse publique d'un nœud. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1'

export type PublicZone = Schemas['PublicZoneDto']
export type DnsSettings = Schemas['DnsSettingsDto']
export type DnsSettingsRequest = Schemas['DnsSettingsRequest']
export type DnsRecord = Schemas['DnsRecordDto']
export type DnsSync = Schemas['DnsSyncDto']
export type DnsTest = Schemas['DnsTestDto']

export function useDnsApi() {
  return {
    zone: () => http.get<PublicZone>(`${BASE}/dns`).then((response) => response.data),

    settings: () => http.get<DnsSettings>(`${BASE}/settings/dns`).then((response) => response.data),

    configure: (body: DnsSettingsRequest) =>
      http.put<DnsSettings>(`${BASE}/settings/dns`, body).then((response) => response.data),

    remove: () => http.delete<void>(`${BASE}/settings/dns`).then(() => undefined),

    test: () => http.post<DnsTest>(`${BASE}/settings/dns/test`).then((response) => response.data),

    sync: () => http.post<DnsSync>(`${BASE}/settings/dns/sync`).then((response) => response.data),

    setServerLabel: (id: string, label: string | null) =>
      http.put<void>(`${BASE}/servers/${id}/dns`, { label } satisfies Schemas['LabelRequest']).then(() => undefined),

    setNodeAddress: (id: string, body: Schemas['AddressRequest']) =>
      http.put<void>(`${BASE}/nodes/${id}/address`, body).then(() => undefined),
  }
}
