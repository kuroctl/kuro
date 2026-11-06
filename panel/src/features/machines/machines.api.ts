// Appels API des nœuds à la demande (ADR-0065) : réglages et identifiants des fournisseurs,
// machines louées, offres d'une région, location, retour, sommeil et réveil.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1'

export type MachinesSettings = Schemas['MachinesSettings']
export type MachinesSettingsRequest = Schemas['MachinesSettingsRequest']
export type MachinesPage = Schemas['MachinesPage']
export type Machine = Schemas['MachineView']
export type Offer = Schemas['Offer']
export type RentRequest = Schemas['RentRequest']
export type Location = Schemas['Location']
export type ProviderSaved = Schemas['ProviderSaved']

export function useMachinesApi() {
  return {
    settings: () => http.get<MachinesSettings>(`${BASE}/settings/machines`).then((response) => response.data),

    saveSettings: (body: MachinesSettingsRequest) =>
      http.put<MachinesSettings>(`${BASE}/settings/machines`, body).then((response) => response.data),

    setProvider: (provider: string, token: string) =>
      http
        .put<ProviderSaved>(`${BASE}/settings/machines/providers/${provider}`, { token } satisfies Schemas['ProviderRequest'])
        .then((response) => response.data),

    createRegions: (provider: string) =>
      http.post<ProviderSaved>(`${BASE}/settings/machines/providers/${provider}/regions`).then((response) => response.data),

    locations: (provider: string) =>
      http.get<Location[]>(`${BASE}/settings/machines/providers/${provider}/locations`).then((response) => response.data),

    removeProvider: (provider: string) =>
      http.delete<MachinesSettings>(`${BASE}/settings/machines/providers/${provider}`).then((response) => response.data),

    list: () => http.get<MachinesPage>(`${BASE}/machines`).then((response) => response.data),

    offers: (regionId: string) =>
      http.get<Offer[]>(`${BASE}/machines/offers`, { params: { region_id: regionId } }).then((response) => response.data),

    rent: (body: RentRequest) => http.post<Machine>(`${BASE}/machines`, body).then((response) => response.data),

    release: (id: string) => http.delete<void>(`${BASE}/machines/${id}`).then(() => undefined),

    sleep: (id: string) => http.post<Machine>(`${BASE}/machines/${id}/sleep`).then((response) => response.data),

    wake: (id: string) => http.post<Machine>(`${BASE}/machines/${id}/wake`).then((response) => response.data),
  }
}
