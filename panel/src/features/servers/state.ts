// États d'un serveur : présentation et rythme de rafraîchissement.
import type { Schemas } from '@/shared/api/types'
import type { BadgeVariants } from '@/shared/ui/badge'

export type ServerState = Schemas['ServerState']

/** États transitoires : l'interface se rafraîchit plus souvent pour les suivre. */
export const TRANSITIONAL: ServerState[] = ['deploying', 'installing', 'restoring', 'starting', 'stopping', 'deleting']

export const FAST_REFRESH_MS = 2000
export const SLOW_REFRESH_MS = 15000

export function isTransitional(state: ServerState): boolean {
  return TRANSITIONAL.includes(state)
}

/** Badge d'un état (design system) : en ligne en vert, transitions pulsées, échec en rouge. */
export const stateBadge: Record<ServerState, { variant: BadgeVariants['variant']; pulse?: boolean }> = {
  running: { variant: 'success' },
  starting: { variant: 'warning', pulse: true },
  stopping: { variant: 'warning', pulse: true },
  deploying: { variant: 'info', pulse: true },
  installing: { variant: 'info', pulse: true },
  restoring: { variant: 'info', pulse: true },
  deleting: { variant: 'destructive', pulse: true },
  offline: { variant: 'secondary' },
  failed: { variant: 'destructive' },
}

/** Actions possibles selon l'état (l'API reste seule juge). */
export function canPower(state: ServerState, action: Schemas['PowerActionDto']): boolean {
  if (isTransitional(state) && action !== 'kill') return false
  switch (action) {
    case 'start':
      return state === 'offline' || state === 'failed'
    case 'stop':
    case 'restart':
      return state === 'running'
    case 'kill':
      return state === 'running' || state === 'starting' || state === 'stopping'
  }
}

const MIB = 1024 * 1024

/** Quota disque en octets ; `null` : illimité (ADR-0019). */
export function diskQuota(server: { disk_mb: number }): number | null {
  return server.disk_mb > 0 ? server.disk_mb * MIB : null
}

/** Quota atteint d'après la dernière mesure du nœud : le nœud refuse écritures et démarrage. */
export function isOverQuota(server: { disk_mb: number; disk_used_bytes?: number | null }): boolean {
  const quota = diskQuota(server)
  return quota !== null && server.disk_used_bytes != null && server.disk_used_bytes >= quota
}

/** La console accepte des commandes (l'API reste seule juge). */
export function canCommand(state: ServerState): boolean {
  return state === 'running' || state === 'starting'
}
