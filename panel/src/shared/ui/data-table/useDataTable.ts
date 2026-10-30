// Table TanStack branchée sur l'état de l'URL (`useTableQuery`). Mode API (`rowCount` fourni) :
// les données sont déjà une page triée et filtrée. Mode local : la table trie et pagine
// elle-même la liste reçue, filtrée au préalable par l'appelant.
import {
  type ColumnDef,
  type ColumnVisibilityState,
  functionalUpdate,
  type RowData,
  type RowSelectionState,
  useTable,
} from '@tanstack/vue-table'
import { computed, type MaybeRefOrGetter, ref, toValue, watch } from 'vue'

import { dataTableFeatures, type DataTableFeatures } from './features'
import type { TableQuery } from './useTableQuery'

export interface DataTableOptions<TData extends RowData> {
  data: MaybeRefOrGetter<TData[]>
  // Valeurs de colonnes hétérogènes : `columnHelper.columns([...])` produit ce type.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  columns: ColumnDef<DataTableFeatures, TData, any>[]
  getRowId: (row: TData) => string
  query: TableQuery
  /** Total côté API : active le mode serveur (tri et pagination faits par l'API). */
  rowCount?: MaybeRefOrGetter<number | undefined>
  /** Colonnes affichées, gardées pour cet écran (`localStorage`). */
  storageKey?: string
  /** Colonnes masquées par défaut. */
  hidden?: string[]
}

function loadVisibility(key: string | undefined, hidden: string[]): ColumnVisibilityState {
  const defaults = Object.fromEntries(hidden.map((id) => [id, false]))
  if (!key) return defaults
  try {
    const stored = localStorage.getItem(`kuro.table.${key}`)
    return stored ? { ...defaults, ...JSON.parse(stored) } : defaults
  } catch {
    return defaults
  }
}

export function useDataTable<TData extends RowData>(options: DataTableOptions<TData>) {
  const manual = options.rowCount !== undefined
  const rowSelection = ref<RowSelectionState>({})
  const columnVisibility = ref<ColumnVisibilityState>(loadVisibility(options.storageKey, options.hidden ?? []))

  watch(columnVisibility, (value) => {
    if (!options.storageKey) return
    try {
      localStorage.setItem(`kuro.table.${options.storageKey}`, JSON.stringify(value))
    } catch {
      // Stockage indisponible (navigation privée) : le choix vaut pour la session.
    }
  })

  const table = useTable({
    features: dataTableFeatures,
    get data(): TData[] {
      return toValue(options.data)
    },
    columns: options.columns,
    getRowId: options.getRowId,
    manualPagination: manual,
    manualSorting: manual,
    get rowCount() {
      return manual ? toValue(options.rowCount) : undefined
    },
    enableSortingRemoval: false,
    // `useTableQuery` ramène déjà à la page 1 quand un critère change ; un simple rechargement
    // des données (Vue Query) ne doit pas faire perdre la page.
    autoResetPageIndex: false,
    state: {
      get sorting() {
        return options.query.sorting.value
      },
      get pagination() {
        return options.query.pagination.value
      },
      get rowSelection() {
        return rowSelection.value
      },
      get columnVisibility() {
        return columnVisibility.value
      },
    },
    onSortingChange: (updater) => options.query.setSorting(functionalUpdate(updater, options.query.sorting.value)),
    onPaginationChange: (updater) => options.query.setPagination(functionalUpdate(updater, options.query.pagination.value)),
    onRowSelectionChange: (updater) => {
      rowSelection.value = functionalUpdate(updater, rowSelection.value)
    },
    onColumnVisibilityChange: (updater) => {
      columnVisibility.value = functionalUpdate(updater, columnVisibility.value)
    },
  })

  // La liste a rétréci (suppression, filtre côté API) sous une page qui n'existe plus : on revient
  // à la dernière page existante plutôt que d'afficher une table vide.
  watch(
    () => [table.getPageCount(), options.query.pagination.value.pageIndex] as const,
    ([count, index]) => {
      if (count > 0 && index >= count) {
        options.query.setPagination({ ...options.query.pagination.value, pageIndex: count - 1 })
      }
    },
  )

  const selectedIds = computed(() => Object.keys(rowSelection.value).filter((id) => rowSelection.value[id]))
  function clearSelection() {
    rowSelection.value = {}
  }

  return { table, rowSelection, selectedIds, clearSelection }
}

export type DataTableInstance<TData extends RowData> = ReturnType<typeof useDataTable<TData>>['table']
