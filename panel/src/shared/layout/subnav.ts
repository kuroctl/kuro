// Sous-menu d'une section (`Section.items`) traduit en entrées du menu latéral (`NavEntry`) :
// permissions, groupes et page active.
import type { NavEntry } from './nav'
import type { Section, SubnavChild, SubnavGroup, SubnavItem } from './sections'

export function isGroup(item: SubnavItem): item is SubnavGroup {
  return item.children !== undefined
}

/** Toutes les destinations du sous-menu, groupes dépliés. */
function destinations(items: SubnavItem[]): string[] {
  return items.flatMap((item) => (isGroup(item) ? item.children.map((child) => child.to) : item.to ? [item.to] : []))
}

/** Entrée active : la page exacte, ou l'une de ses sous-pages qu'aucune entrée plus précise ne
 * revendique. La racine de la section garde ainsi `/users/<id>`, mais pas `/users/roles`, et
 * `/cloud/catalog` laisse `/cloud/catalog/regions` à son entrée. */
export function isActive(to: string | undefined, path: string, all: string[]): boolean {
  if (!to) return false
  if (path === to) return true
  if (!path.startsWith(`${to}/`)) return false
  return !all.some((other) => other.length > to.length && (path === other || path.startsWith(`${other}/`)))
}

export function subnavEntries(section: Section | undefined, path: string, can: (permission: string) => boolean, t: (key: string) => string): NavEntry[] {
  const items = section?.items ?? []
  const all = destinations(items)
  const allowed = (item: { permission?: string }) => !item.permission || can(item.permission)
  return items.filter(allowed).flatMap((item): NavEntry[] => {
    if (!isGroup(item)) {
      return [{ key: item.label, label: t(item.label), icon: item.icon, to: item.to, soon: item.soon, active: isActive(item.to, path, all) }]
    }
    const children = item.children.filter(allowed).map(
      (child: SubnavChild): NavEntry => ({ key: child.label, label: t(child.label), icon: item.icon, to: child.to, active: isActive(child.to, path, all) }),
    )
    return children.length > 0 ? [{ key: item.label, label: t(item.label), icon: item.icon, children }] : []
  })
}
