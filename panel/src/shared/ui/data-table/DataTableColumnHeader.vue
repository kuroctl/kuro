<script setup lang="ts" generic="TData extends RowData, TValue extends CellData">
// En-tête d'une colonne triable : un premier clic trie en croissant (en décroissant si la colonne
// déclare `sortDescFirst`, pour une date), le suivant inverse l'ordre.
import { ArrowDown, ArrowUp, ArrowUpDown } from '@lucide/vue'
import type { CellData, Column, RowData } from '@tanstack/vue-table'

import { Button } from '@/shared/ui/button'

import type { DataTableFeatures } from './features'

defineProps<{ column: Column<DataTableFeatures, TData, TValue>; title: string }>()
</script>

<template>
  <Button
    v-if="column.getCanSort()"
    variant="ghost"
    size="sm"
    class="-ml-2 h-8 gap-1.5 px-2 text-xs font-semibold uppercase tracking-wide"
    :aria-sort="column.getIsSorted() === 'asc' ? 'ascending' : column.getIsSorted() === 'desc' ? 'descending' : 'none'"
    @click="column.toggleSorting(column.getIsSorted() ? column.getIsSorted() === 'asc' : column.columnDef.sortDescFirst === true)"
  >
    {{ title }}
    <ArrowUp v-if="column.getIsSorted() === 'asc'" class="size-3.5" />
    <ArrowDown v-else-if="column.getIsSorted() === 'desc'" class="size-3.5" />
    <ArrowUpDown v-else class="size-3.5 opacity-50" />
  </Button>
  <span v-else class="text-xs font-semibold uppercase tracking-wide">{{ title }}</span>
</template>
