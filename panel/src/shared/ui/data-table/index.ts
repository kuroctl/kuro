// Listes du panel (ADR-0025) : TanStack Table v9, état dans l'URL, mode API ou local.
import type { CellData, HeaderContext, RowData, TableFeatures } from '@tanstack/vue-table'
import { type Component, h } from 'vue'

import DataTableColumnHeader from './DataTableColumnHeader.vue'
import type { DataTableFeatures as Features } from './features'

export { default as DataTable } from './DataTable.vue'
export { default as DataTableColumnHeader } from './DataTableColumnHeader.vue'
export { default as DataTableFacetFilter, type FacetOption } from './DataTableFacetFilter.vue'
export { default as DataTablePagination } from './DataTablePagination.vue'
export { default as DataTableSearch } from './DataTableSearch.vue'
export { default as DataTableSelectionBar } from './DataTableSelectionBar.vue'
export { default as DataTableViewOptions } from './DataTableViewOptions.vue'
export { dataTableFeatures, type DataTableFeatures } from './features'
export { type DataTableInstance, useDataTable } from './useDataTable'
export { PAGE_SIZES, type TableQuery, useTableQuery } from './useTableQuery'

/** En-tête non triable, au même style que les en-têtes triables. */
export function plain(title: string) {
  return () => h('span', { class: 'text-xs font-semibold uppercase tracking-wide' }, title)
}

/** En-tête triable d'une colonne. */
export function sortable<TData extends RowData, TValue extends CellData>(title: string) {
  // Composant générique : `h` ne sait pas en inférer les paramètres de type.
  return ({ column }: HeaderContext<Features, TData, TValue>) => h(DataTableColumnHeader as Component, { column, title })
}

declare module '@tanstack/vue-table' {
  // Paramètres imposés par la déclaration d'origine, inutilisés ici.
  /* eslint-disable @typescript-eslint/no-unused-vars */
  interface ColumnMeta<in out TFeatures extends TableFeatures, in out TData extends RowData, TValue extends CellData = CellData> {
    /** Libellé traduit, pour le choix des colonnes affichées. */
    title?: string
    /** Classes de l'en-tête et des cellules (largeur, alignement). */
    class?: string
  }
  /* eslint-enable @typescript-eslint/no-unused-vars */
}
