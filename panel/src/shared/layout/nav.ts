// Entrées du menu latéral : une page, ou un groupe repliable de pages.
import type { LucideIcon } from '@lucide/vue'

export interface NavEntry {
  key: string
  /** Libellé déjà traduit. */
  label: string
  icon: LucideIcon
  to?: string
  active?: boolean
  /** Fonctionnalité annoncée mais pas encore disponible. */
  soon?: boolean
  children?: NavEntry[]
}

/** Comparaison sans casse ni accents : « securite » trouve « Sécurité ». */
function normalize(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase()
}

/** Entrées qui correspondent à la recherche : un groupe garde ses pages s'il correspond lui-même,
 * sinon seulement celles qui correspondent. */
export function filterEntries(entries: NavEntry[], query: string): NavEntry[] {
  const needle = normalize(query.trim())
  if (!needle) return entries
  return entries.flatMap((entry) => {
    if (normalize(entry.label).includes(needle)) return [entry]
    const children = entry.children?.filter((child) => normalize(child.label).includes(needle)) ?? []
    return children.length > 0 ? [{ ...entry, children }] : []
  })
}
