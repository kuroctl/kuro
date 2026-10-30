<script setup lang="ts" generic="TData extends RowData">
// Corps d'une liste : en-têtes et lignes rendus par TanStack Table (`useDataTable`). Une ligne
// cliquable (`rowTo`) mène à sa fiche, sauf clic sur une case à cocher ou un bouton.
import { FlexRender, type RowData } from '@tanstack/vue-table'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'

import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/shared/ui/table'

import type { DataTableInstance } from './useDataTable'

const props = defineProps<{
  table: DataTableInstance<TData>
  /** Fiche d'une ligne. */
  rowTo?: (row: TData) => string
  loading?: boolean
}>()

const { t } = useI18n()
const router = useRouter()

function open(event: MouseEvent, row: TData) {
  if (!props.rowTo) return
  const target = event.target as HTMLElement
  if (target.closest('button, a, input, [role="checkbox"], [role="menuitem"]')) return
  void router.push(props.rowTo(row))
}
</script>

<template>
  <div class="overflow-hidden rounded-xl border bg-card shadow-card">
    <Table>
      <TableHeader>
        <TableRow v-for="group in table.getHeaderGroups()" :key="group.id" class="hover:bg-transparent">
          <TableHead v-for="header in group.headers" :key="header.id" :class="header.column.columnDef.meta?.class">
            <FlexRender v-if="!header.isPlaceholder" :header="header" />
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <template v-if="table.getRowModel().rows.length">
          <TableRow
            v-for="row in table.getRowModel().rows"
            :key="row.id"
            :data-state="row.getIsSelected() ? 'selected' : undefined"
            :class="rowTo ? 'cursor-pointer' : undefined"
            @click="open($event, row.original)"
          >
            <TableCell v-for="cell in row.getVisibleCells()" :key="cell.id" :class="cell.column.columnDef.meta?.class">
              <FlexRender :cell="cell" />
            </TableCell>
          </TableRow>
        </template>
        <TableRow v-else class="hover:bg-transparent">
          <TableCell :colspan="table.getVisibleLeafColumns().length" class="h-24 text-center text-muted-foreground">
            {{ loading ? t('common.loading') : t('dataTable.empty') }}
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>
  </div>
</template>
