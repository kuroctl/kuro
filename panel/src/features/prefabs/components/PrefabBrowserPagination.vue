<script setup lang="ts">
// Pagination d'un catalogue de prefabs : plage affichée, numéros de page sur grand écran
// (premier, dernier et voisins), « Page x sur y » sur mobile. Remonte en haut de la liste.
import { ChevronLeft, ChevronRight, MoreHorizontal } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'

import type { PrefabBrowser } from '../composables/usePrefabBrowser'

const props = defineProps<{ browser: PrefabBrowser; scrollTarget?: HTMLElement | null }>()
const { t } = useI18n()
const { page, pageCount, range } = props.browser

/** Numéros affichés ; `null` pour un saut. */
const pages = computed(() => {
  const shown = [...new Set([0, page.value - 1, page.value, page.value + 1, pageCount.value - 1])]
    .filter((index) => index >= 0 && index < pageCount.value)
    .sort((a, b) => a - b)
  return shown.flatMap((index, i) => (i > 0 && index - shown[i - 1]! > 1 ? [null, index] : [index]))
})

function go(index: number) {
  props.browser.setPage(index)
  props.scrollTarget?.scrollIntoView?.({ block: 'start', behavior: 'smooth' })
}
</script>

<template>
  <nav
    v-if="pageCount > 1"
    class="flex flex-wrap items-center justify-between gap-3 text-sm text-muted-foreground"
    :aria-label="t('prefabs.browser.pagination')"
  >
    <span>{{ t('prefabs.browser.range', range) }}</span>
    <div class="flex items-center gap-1">
      <Button variant="outline" size="icon-sm" :aria-label="t('dataTable.previous')" :disabled="page === 0" @click="go(page - 1)">
        <ChevronLeft />
      </Button>
      <template v-for="(index, i) in pages" :key="index ?? `gap-${i}`">
        <MoreHorizontal v-if="index === null" class="hidden size-4 w-6 sm:inline" aria-hidden="true" />
        <Button
          v-else
          size="icon-sm"
          class="hidden tabular-nums sm:inline-flex"
          :variant="index === page ? 'default' : 'ghost'"
          :aria-current="index === page ? 'page' : undefined"
          :aria-label="t('prefabs.browser.goToPage', { page: index + 1 })"
          @click="go(index)"
        >
          {{ index + 1 }}
        </Button>
      </template>
      <span class="px-2 tabular-nums sm:hidden">{{ t('dataTable.page', { page: page + 1, count: pageCount }) }}</span>
      <Button variant="outline" size="icon-sm" :aria-label="t('dataTable.next')" :disabled="page >= pageCount - 1" @click="go(page + 1)">
        <ChevronRight />
      </Button>
    </div>
  </nav>
</template>
