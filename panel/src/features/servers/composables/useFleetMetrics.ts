// Indicateurs de la vue d'ensemble, calculés depuis les listes des serveurs et des nœuds : pas
// d'agrégat côté API tant que ces listes restent de taille raisonnable.
import { computed, type Ref } from 'vue'

import type { Node } from '@/features/nodes/nodes.api'

import type { Server } from '../servers.api'
import { diskQuota, isOverQuota } from '../state'

const MIB = 1024 * 1024

export function useFleetMetrics(servers: Ref<Server[] | undefined>, nodes: Ref<Node[] | undefined>) {
  return computed(() => {
    const list = servers.value ?? []
    const hosts = nodes.value ?? []
    const online = hosts.filter((node) => node.status === 'online')
    const capacity = online.reduce((sum, node) => sum + (node.host?.memory_bytes ?? 0), 0)
    const quotas = list.map(diskQuota).filter((quota): quota is number => quota !== null)
    return {
      servers: list.length,
      running: list.filter((server) => server.state === 'running').length,
      attention: list.filter((server) => server.state === 'failed' || !!server.suspended_at || isOverQuota(server)).length,
      nodes: hosts.length,
      nodesOnline: online.length,
      nodesPending: hosts.filter((node) => node.status === 'pending').length,
      memoryAllocated: list.reduce((sum, server) => sum + server.memory_mb * MIB, 0),
      /** Mémoire des nœuds en ligne ; `null` si aucun n'a remonté sa capacité. */
      memoryCapacity: capacity > 0 ? capacity : null,
      diskUsed: list.reduce((sum, server) => sum + (server.disk_used_bytes ?? 0), 0),
      /** Somme des quotas ; `null` si aucun serveur n'a de quota. */
      diskQuota: quotas.length > 0 ? quotas.reduce((sum, quota) => sum + quota, 0) : null,
      /** Serveurs par nœud, pour les bulles de la carte. */
      perNode: list.reduce<Record<string, number>>((counts, server) => {
        counts[server.node.id] = (counts[server.node.id] ?? 0) + 1
        return counts
      }, {}),
      unplaced: hosts.filter((node) => !node.map_position).length,
    }
  })
}
