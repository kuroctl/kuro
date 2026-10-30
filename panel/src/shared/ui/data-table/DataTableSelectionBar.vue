<script setup lang="ts">
// Sélection en cours et ses actions. Toute la page cochée : on propose d'étendre la sélection
// à tous les résultats du filtre, pages suivantes comprises.
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'

defineProps<{
  /** Lignes cochées sur la page. */
  selected: number
  /** Toute la page est cochée. */
  pageSelected: boolean
  /** Résultats du filtre, toutes pages confondues. */
  total: number
  /** La sélection couvre tous les résultats. */
  allMatching: boolean
}>()
const emit = defineEmits<{ selectAll: []; clear: [] }>()
const { t } = useI18n()
</script>

<template>
  <div
    v-if="selected > 0 || allMatching"
    class="flex flex-wrap items-center gap-3 rounded-xl border border-primary/30 bg-primary/8 px-4 py-2.5 text-sm"
    role="region"
    :aria-label="t('dataTable.selection')"
  >
    <span class="font-medium">
      {{ allMatching ? t('dataTable.allSelected', { count: total }) : t('dataTable.selected', { count: selected }) }}
    </span>
    <Button v-if="pageSelected && !allMatching && total > selected" variant="link" size="sm" class="h-auto p-0" @click="emit('selectAll')">
      {{ t('dataTable.selectAll', { count: total }) }}
    </Button>
    <Button variant="link" size="sm" class="h-auto p-0 text-muted-foreground" @click="emit('clear')">{{ t('dataTable.clearSelection') }}</Button>
    <div class="ml-auto flex flex-wrap items-center gap-2"><slot /></div>
  </div>
</template>
