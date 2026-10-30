<script setup lang="ts" generic="TData extends RowData">
// Pagination : nombre de résultats, taille de page, page courante et navigation.
import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight } from '@lucide/vue'
import type { RowData } from '@tanstack/vue-table'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'
import { NativeSelect, NativeSelectOption } from '@/shared/ui/native-select'

import type { DataTableInstance } from './useDataTable'
import { PAGE_SIZES, type TableQuery } from './useTableQuery'

const props = defineProps<{ table: DataTableInstance<TData>; query: TableQuery; total: number }>()
const { t } = useI18n()

const size = computed({
  get: () => String(props.query.pagination.value.pageSize),
  set: (value: string) => props.query.setPagination({ pageIndex: 0, pageSize: Number(value) }),
})
</script>

<template>
  <div class="flex flex-wrap items-center justify-between gap-3 px-1 text-sm text-muted-foreground">
    <span>{{ t('dataTable.total', { count: total }) }}</span>
    <div class="flex flex-wrap items-center gap-4">
      <label class="flex items-center gap-2">
        {{ t('dataTable.perPage') }}
        <NativeSelect v-model="size" class="h-8 w-20">
          <NativeSelectOption v-for="option in PAGE_SIZES" :key="option" :value="String(option)">{{ option }}</NativeSelectOption>
        </NativeSelect>
      </label>
      <span>{{ t('dataTable.page', { page: query.pagination.value.pageIndex + 1, count: Math.max(1, table.getPageCount()) }) }}</span>
      <div class="flex items-center gap-1">
        <Button variant="outline" size="icon" class="size-8" :aria-label="t('dataTable.first')" :disabled="!table.getCanPreviousPage()" @click="table.firstPage()">
          <ChevronsLeft />
        </Button>
        <Button variant="outline" size="icon" class="size-8" :aria-label="t('dataTable.previous')" :disabled="!table.getCanPreviousPage()" @click="table.previousPage()">
          <ChevronLeft />
        </Button>
        <Button variant="outline" size="icon" class="size-8" :aria-label="t('dataTable.next')" :disabled="!table.getCanNextPage()" @click="table.nextPage()">
          <ChevronRight />
        </Button>
        <Button variant="outline" size="icon" class="size-8" :aria-label="t('dataTable.last')" :disabled="!table.getCanNextPage()" @click="table.lastPage()">
          <ChevronsRight />
        </Button>
      </div>
    </div>
  </div>
</template>
