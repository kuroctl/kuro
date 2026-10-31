// Appels API des nœuds et de leurs ports. Aucune logique ici : les erreurs remontent à l'appelant.
import { http } from '@/shared/api/http'
import type { Schemas } from '@/shared/api/types'

const BASE = '/admin/v1/nodes'

export type Node = Schemas['NodeDto']
export type Allocation = Schemas['AllocationDto']
export type NodeTicket = Schemas['NodeTicketDto']

export function useNodesApi() {
  return {
    list: () => http.get<Node[]>(BASE).then((response) => response.data),

    create: (body: Schemas['CreateNodeRequest']) => http.post<NodeTicket>(BASE, body).then((response) => response.data),

    reissueToken: (id: string) => http.post<NodeTicket>(`${BASE}/${id}/enrollment-token`).then((response) => response.data),

    get: (id: string) => http.get<Node>(`${BASE}/${id}`).then((response) => response.data),

    update: (id: string, body: Schemas['UpdateNodeRequest']) => http.patch<Node>(`${BASE}/${id}`, body).then((response) => response.data),

    remove: (id: string) => http.delete<void>(`${BASE}/${id}`).then(() => undefined),

    allocations: (id: string) => http.get<Allocation[]>(`${BASE}/${id}/allocations`).then((response) => response.data),

    addAllocations: (id: string, body: Schemas['CreateAllocationsRequest']) =>
      http.post<Allocation[]>(`${BASE}/${id}/allocations`, body).then((response) => response.data),

    removeAllocation: (id: string, allocationId: string) =>
      http.delete<void>(`${BASE}/${id}/allocations/${allocationId}`).then(() => undefined),
  }
}
