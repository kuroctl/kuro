// Ressources minimales et recommandées d'un prefab (ADR-0031), lues dans le résumé du catalogue.
import type { Schemas } from '@/shared/api/types'

export type NeededResource = 'memory_mb' | 'cpu_percent' | 'disk_mb'
export const NEEDED_RESOURCES: NeededResource[] = ['memory_mb', 'cpu_percent', 'disk_mb']

type Entry = Pick<Schemas['PrefabSummaryDto'], 'resources'> | undefined

export function need(entry: Entry, resource: NeededResource): Schemas['ResourceNeedDto'] | undefined {
  return entry?.resources[resource]
}

/**
 * Valeur proposée à la création : la plus haute entre le défaut du projet et la recommandation
 * du prefab, ramenée au plafond.
 */
export function prefill(entry: Entry, resource: NeededResource, projectDefault: number, ceiling?: number | null): number {
  const recommended = need(entry, resource)?.recommended ?? 0
  const value = Math.max(projectDefault, recommended)
  return ceiling != null ? Math.min(value, ceiling) : value
}

/** Ressources du serveur sous le minimum de son prefab (disque `0` : illimité, jamais sous). */
export function belowMinimum(entry: Entry, server: Record<NeededResource, number>): NeededResource[] {
  return NEEDED_RESOURCES.filter((resource) => {
    const min = need(entry, resource)?.min
    const unlimited = resource === 'disk_mb' && server.disk_mb === 0
    return min != null && !unlimited && server[resource] < min
  })
}
