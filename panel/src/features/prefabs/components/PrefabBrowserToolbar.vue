<script setup lang="ts">
// Barre d'un catalogue de prefabs : recherche (raccourci `/`), tags, origine, tri, vue en grille
// ou en liste, catégories (défilantes sur mobile), puis nombre de résultats.
import { LayoutGrid, List, Search, X } from '@lucide/vue'
import { onKeyStroke, useDebounceFn } from '@vueuse/core'
import { computed, ref, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { Button } from '@/shared/ui/button'
import { DataTableFacetFilter } from '@/shared/ui/data-table'
import { Input } from '@/shared/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/shared/ui/select'

import { type PrefabBrowser, SORTS } from '../composables/usePrefabBrowser'

/** Touche qui place le curseur dans la recherche. */
const SHORTCUT = '/'

const props = defineProps<{ browser: PrefabBrowser; sourceFilter?: boolean }>()
const view = defineModel<'grid' | 'list'>('view', { required: true })
const { t, te } = useI18n()
const { query, category, categories, categoryTotal, tags, tagOptions, sources, sourceOptions, sort, results } = props.browser

// Recherche : appliquée après une courte pause.
const search = useTemplateRef<HTMLElement>('search')
const draft = ref(query.q.value)
watch(query.q, (value) => {
  if (value !== draft.value.trim()) draft.value = value
})
const applySearch = useDebounceFn((value: string) => (query.q.value = value), 150)
onKeyStroke(SHORTCUT, (event) => {
  const target = event.target as HTMLElement | null
  if (target?.closest('input, textarea, select, [contenteditable="true"]')) return
  event.preventDefault()
  search.value?.querySelector('input')?.focus()
})

const tagFacet = computed(() => tagOptions.value.map((option) => ({ ...option, label: option.value })))
const sourceLabel = (source: string) => (te(`prefabs.catalog.${source}`) ? t(`prefabs.catalog.${source}`) : source)
const sourceFacet = computed(() => sourceOptions.value.map((option) => ({ ...option, label: sourceLabel(option.value) })))
</script>

<template>
  <div class="grid gap-4">
    <div class="flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center">
      <div ref="search" class="relative w-full sm:max-w-sm sm:flex-1">
        <Search class="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          v-model="draft"
          type="search"
          :placeholder="t('prefabs.browser.search')"
          :aria-label="t('prefabs.browser.search')"
          class="h-8 pl-8 sm:pr-9"
          @update:model-value="(value) => applySearch(String(value))"
        />
        <kbd
          class="pointer-events-none absolute top-1/2 right-2.5 hidden -translate-y-1/2 rounded border bg-muted px-1.5 font-mono text-[11px] text-muted-foreground sm:block"
          :title="t('prefabs.browser.shortcut')"
        >{{ SHORTCUT }}</kbd>
      </div>
      <div class="flex flex-wrap items-center gap-2">
        <DataTableFacetFilter :model-value="tags" :title="t('prefabs.browser.tags')" :options="tagFacet" @update:model-value="browser.setTags" />
        <DataTableFacetFilter
          v-if="sourceFilter && (sourceFacet.length > 1 || sources.length)"
          :model-value="sources"
          :title="t('prefabs.browser.source')"
          :options="sourceFacet"
          @update:model-value="browser.setSources"
        />
        <Select :model-value="sort" @update:model-value="browser.setSort">
          <SelectTrigger size="sm" class="min-w-0 flex-1 sm:w-48 sm:flex-none" :aria-label="t('prefabs.browser.sort.label')">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem v-for="option in SORTS" :key="option" :value="option">{{ t(`prefabs.browser.sort.${option}`) }}</SelectItem>
          </SelectContent>
        </Select>
        <div class="flex shrink-0 rounded-md border p-px" role="group" :aria-label="t('prefabs.browser.view.label')">
          <Button
            v-for="option in (['grid', 'list'] as const)"
            :key="option"
            size="icon-sm"
            class="size-7"
            :variant="view === option ? 'secondary' : 'ghost'"
            :aria-label="t(`prefabs.browser.view.${option}`)"
            :aria-pressed="view === option"
            :title="t(`prefabs.browser.view.${option}`)"
            @click="view = option"
          >
            <LayoutGrid v-if="option === 'grid'" /><List v-else />
          </Button>
        </div>
      </div>
    </div>

    <!-- Catégories : défilent horizontalement sur mobile. -->
    <div
      v-if="categories.length > 1 || category"
      class="-mx-6 flex gap-2 overflow-x-auto px-6 pb-1 [scrollbar-width:none] sm:mx-0 sm:flex-wrap sm:px-0"
      role="group"
      :aria-label="t('prefabs.browser.categories')"
    >
      <Button
        size="sm"
        class="shrink-0 rounded-full"
        :variant="category ? 'outline' : 'default'"
        :aria-pressed="!category"
        @click="browser.setCategory(undefined)"
      >
        {{ t('prefabs.browser.all') }}<span class="text-xs opacity-70">{{ categoryTotal }}</span>
      </Button>
      <Button
        v-for="option in categories"
        :key="option.value"
        size="sm"
        class="shrink-0 rounded-full"
        :variant="category === option.value ? 'default' : 'outline'"
        :aria-pressed="category === option.value"
        @click="browser.setCategory(category === option.value ? undefined : option.value)"
      >
        {{ t(`prefabs.catalog.categories.${option.value}`) }}<span class="text-xs opacity-70">{{ option.count }}</span>
      </Button>
    </div>

    <div v-if="results.length" class="flex items-center justify-between gap-2 text-sm text-muted-foreground" aria-live="polite">
      <span>{{ t('prefabs.browser.count', results.length) }}</span>
      <Button v-if="query.active.value" variant="ghost" size="sm" @click="browser.reset">{{ t('dataTable.reset') }}<X /></Button>
    </div>
  </div>
</template>
