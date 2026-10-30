// Catalogue des prefabs : une entrée par prefab (la version à afficher ou à déployer, ses
// serveurs), et la recherche textuelle (insensible à la casse et aux accents) sur ses noms, son
// id, ses tags et sa description.
import type { Schemas } from '@/shared/api/types'

type Summary = Schemas['PrefabSummaryDto']

export interface CatalogEntry {
  prefab: Summary
  /** Serveurs déployés, toutes versions confondues. */
  servers: number
  /** Versions importées. */
  versions: number
}

export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.split(/[.-]/).map((p) => Number.parseInt(p, 10) || 0)
  const [x, y] = [parts(a), parts(b)]
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) - (y[i] ?? 0)
  }
  return 0
}

/**
 * Dernière version déployable de chaque prefab : une version publiée de préférence, sinon un
 * brouillon (`x-status`), qui ne se déploie que sur un nœud de test (ADR-0011).
 */
export function latestDeployable(prefabs: readonly Summary[]): CatalogEntry[] {
  return group(prefabs).flatMap((versions) => {
    const prefab = versions.filter((entry) => entry.deployable).reduce<Summary | undefined>((current, entry) => {
      const better =
        !current ||
        (current.test_nodes_only && !entry.test_nodes_only) ||
        (current.test_nodes_only === entry.test_nodes_only && compareVersions(entry.version, current.version) > 0)
      return better ? entry : current
    }, undefined)
    return prefab ? [entry(prefab, versions)] : []
  })
}

/** Dernière version importée de chaque prefab, déployable ou non : le catalogue d'administration. */
export function latestImported(prefabs: readonly Summary[]): CatalogEntry[] {
  return group(prefabs).map((versions) => entry(versions.reduce((a, b) => (b.imported_at > a.imported_at ? b : a)), versions))
}

/** Versions regroupées par prefab. */
function group(prefabs: readonly Summary[]): Summary[][] {
  const byId = new Map<string, Summary[]>()
  for (const prefab of prefabs) byId.set(prefab.id, [...(byId.get(prefab.id) ?? []), prefab])
  return [...byId.values()]
}

function entry(prefab: Summary, versions: Summary[]): CatalogEntry {
  return { prefab, servers: versions.reduce((sum, v) => sum + v.servers, 0), versions: versions.length }
}

/** Texte comparable : minuscules, sans accents. */
export function normalize(text: string): string {
  return text.normalize('NFD').replace(/\p{Diacritic}/gu, '').toLowerCase()
}

/** Vrai si chaque mot de la recherche figure dans le prefab (noms de toutes les langues compris). */
export function matches(prefab: Summary, query: string, locale: string): boolean {
  const words = normalize(query).split(/\s+/).filter(Boolean)
  if (!words.length) return true
  const haystack = normalize(
    [
      prefab.id,
      ...Object.values(prefab.names),
      ...prefab.tags,
      prefab.descriptions[locale] ?? prefab.descriptions.en ?? '',
    ].join(' '),
  )
  return words.every((word) => haystack.includes(word))
}
