// État d'une liste dans l'URL : recherche, tri, page, taille de page et filtres. Une vue
// filtrée se partage par lien et survit au rechargement ; `params` la traduit pour l'API.
import type { PaginationState, SortingState } from '@tanstack/vue-table'
import { computed } from 'vue'
import { type LocationQuery, useRoute, useRouter } from 'vue-router'

export interface TableQueryOptions {
  /** Tri par défaut (identifiant de colonne, aussi clé de tri de l'API). */
  sort: string
  order?: 'asc' | 'desc'
  perPage?: number
  /** Filtres à valeurs multiples, écrits `cle=a,b` dans l'URL. */
  filters?: readonly string[]
}

export const PAGE_SIZES = [10, 25, 50, 100] as const

export function useTableQuery(options: TableQueryOptions) {
  const route = useRoute()
  const router = useRouter()
  const perPage = options.perPage ?? 25
  const filterKeys = options.filters ?? []

  const read = (key: string) => (typeof route.query[key] === 'string' ? (route.query[key] as string) : undefined)

  /** Applique des changements ; un changement de critère ramène à la première page. */
  function patch(changes: Record<string, string | undefined>, resetPage = true) {
    const query: LocationQuery = { ...route.query }
    for (const [key, value] of Object.entries(changes)) {
      if (value === undefined || value === '') delete query[key]
      else query[key] = value
    }
    if (resetPage) delete query.page
    void router.replace({ query })
  }

  const q = computed({
    get: () => read('q') ?? '',
    set: (value: string) => patch({ q: value.trim() || undefined }),
  })

  const sorting = computed<SortingState>(() => [
    { id: read('sort') ?? options.sort, desc: (read('order') ?? options.order ?? 'asc') === 'desc' },
  ])
  function setSorting(next: SortingState) {
    const first = next[0]
    patch({ sort: first?.id, order: first ? (first.desc ? 'desc' : 'asc') : undefined })
  }

  const pagination = computed<PaginationState>(() => ({
    pageIndex: Math.max(0, Number(read('page') ?? 1) - 1),
    pageSize: Number(read('size') ?? perPage),
  }))
  function setPagination(next: PaginationState) {
    const sizeChanged = next.pageSize !== pagination.value.pageSize
    patch(
      {
        page: !sizeChanged && next.pageIndex > 0 ? String(next.pageIndex + 1) : undefined,
        size: next.pageSize !== perPage ? String(next.pageSize) : undefined,
      },
      sizeChanged,
    )
  }

  const filters = computed<Record<string, string[]>>(() =>
    Object.fromEntries(filterKeys.map((key) => [key, (read(key) ?? '').split(',').filter(Boolean)])),
  )
  function setFilter(key: string, values: string[]) {
    patch({ [key]: values.join(',') || undefined })
  }

  const active = computed(() => Boolean(q.value) || Object.values(filters.value).some((values) => values.length > 0))
  function reset() {
    patch(Object.fromEntries(['q', ...filterKeys].map((key) => [key, undefined])))
  }

  /** Paramètres de l'API : `q`, `sort`, `order`, `page`, `per_page` et chaque filtre. */
  const params = computed(() => ({
    q: q.value || undefined,
    sort: sorting.value[0]?.id,
    order: sorting.value[0]?.desc ? 'desc' : 'asc',
    page: pagination.value.pageIndex + 1,
    per_page: pagination.value.pageSize,
    ...Object.fromEntries(Object.entries(filters.value).map(([key, values]) => [key, values.join(',') || undefined])),
  }))

  return { q, sorting, setSorting, pagination, setPagination, filters, setFilter, active, reset, params }
}

export type TableQuery = ReturnType<typeof useTableQuery>
