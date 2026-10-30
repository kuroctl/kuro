<script setup lang="ts" generic="TData extends RowData">
// Colonnes affichées ; le choix est gardé pour l'écran (`storageKey` de `useDataTable`).
import { Settings2 } from '@lucide/vue'
import type { RowData } from '@tanstack/vue-table'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/shared/ui/dropdown-menu'

import type { DataTableInstance } from './useDataTable'

const props = defineProps<{ table: DataTableInstance<TData> }>()
const { t } = useI18n()

const columns = computed(() => props.table.getAllLeafColumns().filter((column) => column.getCanHide()))
</script>

<template>
  <DropdownMenu>
    <DropdownMenuTrigger as-child>
      <Button variant="outline" size="sm" class="ml-auto h-8">
        <Settings2 class="size-4" />{{ t('dataTable.columns') }}
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end" class="w-48">
      <DropdownMenuLabel>{{ t('dataTable.columns') }}</DropdownMenuLabel>
      <DropdownMenuSeparator />
      <DropdownMenuCheckboxItem
        v-for="column in columns"
        :key="column.id"
        :model-value="column.getIsVisible()"
        @update:model-value="(visible: boolean) => column.toggleVisibility(visible)"
        @select.prevent
      >
        {{ column.columnDef.meta?.title ?? column.id }}
      </DropdownMenuCheckboxItem>
    </DropdownMenuContent>
  </DropdownMenu>
</template>
