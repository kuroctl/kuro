// Présentation de l'état d'un nœud : variante de badge et couleur du point sur la carte.
import type { BadgeVariants } from '@/shared/ui/badge'

export const NODE_STATUSES = ['online', 'offline', 'pending', 'revoked'] as const

export const nodeBadge: Record<string, BadgeVariants['variant']> = {
  online: 'success',
  offline: 'secondary',
  pending: 'warning',
  revoked: 'destructive',
}

export const nodeDot: Record<string, string> = {
  online: 'bg-success',
  offline: 'bg-muted-foreground',
  pending: 'bg-warning',
  revoked: 'bg-destructive',
}
