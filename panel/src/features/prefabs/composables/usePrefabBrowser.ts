// Parcours d'un catalogue de prefabs arrivé en entier : recherche, catégorie, tags, origine, tri
// et pagination, faits ici. Tout l'état tient dans l'URL (une vue filtrée se partage par lien).
// Partagé par le catalogue (`/prefabs`) et le choix du prefab à la création d'un serveur.
import { computed, type MaybeRefOrGetter, toValue } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Schemas } from '@/shared/api/types'
import { useTableQuery } from '@/shared/ui/data-table'

import { type CatalogEntry, matches } from '../catalog'

/** Multiple de 2, 3 et 4 : pages pleines quel que soit le nombre de colonnes. */
export const PAGE_SIZE = 24
export const SORTS = ['popular', 'name', 'recent'] as const
export type PrefabSort = (typeof SORTS)[number]

export function usePrefabBrowser(entries: MaybeRefOrGetter<readonly CatalogEntry[]>, options: { sort?: PrefabSort } = {}) {
  const { locale } = useI18n()
  const defaultSort = options.sort ?? 'popular'
  const query = useTableQuery({ sort: defaultSort, filters: ['category', 'tags', 'source'], perPage: PAGE_SIZE })

  const name = (prefab: Schemas['PrefabSummaryDto']) => prefab.names[locale.value] ?? prefab.names.en ?? prefab.id

  const category = computed(() => query.filters.value.category?.[0])
  const tags = computed(() => query.filters.value.tags ?? [])
  const sources = computed(() => query.filters.value.source ?? [])

  // Chaque facette compte les résultats des autres critères : ses nombres disent ce qu'on obtiendrait.
  const searched = computed(() => toValue(entries).filter((entry) => matches(entry.prefab, query.q.value, locale.value)))
  const inCategory = (entry: CatalogEntry) => !category.value || entry.prefab.category === category.value
  const withTags = (entry: CatalogEntry) => !tags.value.length || entry.prefab.tags.some((tag) => tags.value.includes(tag))
  const fromSource = (entry: CatalogEntry) => !sources.value.length || sources.value.includes(entry.prefab.source)

  /** Valeurs d'une facette et leur nombre ; une valeur choisie reste proposée, même à zéro. */
  function facet(list: CatalogEntry[], values: (entry: CatalogEntry) => string[], selected: string[]) {
    const counts = new Map<string, number>()
    for (const entry of list) for (const value of values(entry)) counts.set(value, (counts.get(value) ?? 0) + 1)
    for (const value of selected) if (!counts.has(value)) counts.set(value, 0)
    return [...counts].map(([value, count]) => ({ value, count })).sort((a, b) => b.count - a.count || a.value.localeCompare(b.value))
  }
  const forCategories = computed(() => searched.value.filter((entry) => withTags(entry) && fromSource(entry)))
  const categories = computed(() => facet(forCategories.value, (entry) => [entry.prefab.category], category.value ? [category.value] : []))
  const categoryTotal = computed(() => forCategories.value.length)
  const tagOptions = computed(() =>
    facet(searched.value.filter((entry) => inCategory(entry) && fromSource(entry)), (entry) => entry.prefab.tags, tags.value),
  )
  const sourceOptions = computed(() =>
    facet(searched.value.filter((entry) => inCategory(entry) && withTags(entry)), (entry) => [entry.prefab.source], sources.value),
  )

  const sort = computed<PrefabSort>(() => {
    const id = query.sorting.value[0]?.id
    return SORTS.includes(id as PrefabSort) ? (id as PrefabSort) : defaultSort
  })
  function setSort(value: unknown) {
    query.setSorting([{ id: String(value), desc: false }])
  }

  const results = computed(() => {
    const filtered = searched.value.filter((entry) => inCategory(entry) && withTags(entry) && fromSource(entry))
    const byName = (a: CatalogEntry, b: CatalogEntry) => name(a.prefab).localeCompare(name(b.prefab), locale.value)
    const order: Record<PrefabSort, (a: CatalogEntry, b: CatalogEntry) => number> = {
      popular: (a, b) => b.servers - a.servers || byName(a, b),
      name: byName,
      recent: (a, b) => b.prefab.imported_at.localeCompare(a.prefab.imported_at) || byName(a, b),
    }
    return filtered.sort(order[sort.value])
  })

  const pageCount = computed(() => Math.max(1, Math.ceil(results.value.length / PAGE_SIZE)))
  const page = computed(() => Math.min(query.pagination.value.pageIndex, pageCount.value - 1))
  const visible = computed(() => results.value.slice(page.value * PAGE_SIZE, (page.value + 1) * PAGE_SIZE))
  const range = computed(() => ({
    from: page.value * PAGE_SIZE + 1,
    to: Math.min((page.value + 1) * PAGE_SIZE, results.value.length),
    total: results.value.length,
  }))
  function setPage(index: number) {
    query.setPagination({ pageIndex: index, pageSize: PAGE_SIZE })
  }

  return {
    query,
    category,
    tags,
    sources,
    categories,
    categoryTotal,
    tagOptions,
    sourceOptions,
    sort,
    setSort,
    setCategory: (value: string | undefined) => query.setFilter('category', value ? [value] : []),
    setTags: (values: string[]) => query.setFilter('tags', values),
    setSources: (values: string[]) => query.setFilter('source', values),
    reset: () => query.reset(),
    results,
    visible,
    page,
    pageCount,
    range,
    setPage,
  }
}

export type PrefabBrowser = ReturnType<typeof usePrefabBrowser>
