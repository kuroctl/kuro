<script setup lang="ts">
// Catalogue de prefabs parcourable : barre (recherche, filtres, tri, vue), grille de cartes ou
// liste, pagination. Chaque prefab est un lien (`link`) : vers sa fiche dans le catalogue, vers
// l'étape suivante à la création d'un serveur.
import { Boxes, SearchX, X } from '@lucide/vue'
import { useLocalStorage } from '@vueuse/core'
import { useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'
import type { RouteLocationRaw } from 'vue-router'

import { Button } from '@/shared/ui/button'
import { Empty } from '@/shared/ui/empty'
import { Skeleton } from '@/shared/ui/skeleton'

import type { CatalogEntry } from '../catalog'
import { type PrefabSort, usePrefabBrowser } from '../composables/usePrefabBrowser'
import PrefabBrowserPagination from './PrefabBrowserPagination.vue'
import PrefabBrowserToolbar from './PrefabBrowserToolbar.vue'
import PrefabCard from './PrefabCard.vue'
import PrefabRow from './PrefabRow.vue'

const props = defineProps<{
  entries: CatalogEntry[]
  loading?: boolean
  /** Catalogue vide (avant tout filtre). */
  emptyText: string
  /** Clé de la vue (grille ou liste) gardée pour cet écran. */
  viewKey: string
  sort?: PrefabSort
  sourceFilter?: boolean
  link: (entry: CatalogEntry) => RouteLocationRaw
}>()

const { t } = useI18n()
const browser = usePrefabBrowser(() => props.entries, { sort: props.sort })
const { results, visible } = browser
const view = useLocalStorage<'grid' | 'list'>(`kuro.${props.viewKey}.view`, 'grid')
const top = useTemplateRef<HTMLElement>('top')
</script>

<template>
  <div ref="top" class="grid scroll-mt-6 gap-4">
    <PrefabBrowserToolbar v-model:view="view" :browser="browser" :source-filter="sourceFilter" />

    <div v-if="loading" class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
      <Skeleton v-for="index in 6" :key="index" class="h-56 rounded-xl" />
    </div>
    <Empty v-else-if="entries.length === 0" :icon="Boxes">{{ emptyText }}</Empty>
    <Empty v-else-if="results.length === 0" :icon="SearchX" :title="t('prefabs.browser.noMatch')">
      {{ t('prefabs.browser.noMatchHelp') }}
      <template #action>
        <Button variant="outline" size="sm" @click="browser.reset"><X />{{ t('dataTable.reset') }}</Button>
      </template>
    </Empty>

    <template v-else>
      <ul v-if="view === 'grid'" class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
        <li v-for="entry in visible" :key="entry.prefab.id" :data-prefab="entry.prefab.id">
          <RouterLink
            :to="link(entry)"
            class="block h-full rounded-xl transition-transform hover:-translate-y-0.5 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
          >
            <PrefabCard :prefab="entry.prefab" :versions="entry.versions" :servers="entry.servers" />
          </RouterLink>
        </li>
      </ul>
      <ul v-else class="divide-y overflow-hidden rounded-xl border bg-card shadow-card">
        <li v-for="entry in visible" :key="entry.prefab.id" :data-prefab="entry.prefab.id">
          <RouterLink
            :to="link(entry)"
            class="block transition-colors hover:bg-muted/60 focus-visible:bg-muted/60 focus-visible:outline-none"
          >
            <PrefabRow :prefab="entry.prefab" :versions="entry.versions" :servers="entry.servers" />
          </RouterLink>
        </li>
      </ul>

      <PrefabBrowserPagination :browser="browser" :scroll-target="top" />
    </template>
  </div>
</template>
