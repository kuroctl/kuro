<script setup lang="ts">
// Carte d'un prefab dans le catalogue : bannière en fond, vignette portrait (jaquette) si le
// prefab en a une, sinon son icône ; nom, description, badges, versions et serveurs.
import { Server } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import PrefabBlockedBadge from '@/features/prefabs/components/PrefabBlockedBadge.vue'
import type { Schemas } from '@/shared/api/types'
import { Badge } from '@/shared/ui/badge'

import { usePrefabAsset } from '../prefabs.queries'
import PrefabCover from './PrefabCover.vue'
import PrefabIcon from './PrefabIcon.vue'

const props = defineProps<{ prefab: Schemas['PrefabSummaryDto']; versions: number; servers: number }>()
const { t, locale } = useI18n()

const thumbnail = usePrefabAsset(() => props.prefab, 'thumbnail')
const name = computed(() => props.prefab.names[locale.value] ?? props.prefab.names.en ?? props.prefab.id)
const description = computed(() => props.prefab.descriptions[locale.value] ?? props.prefab.descriptions.en)
</script>

<template>
  <PrefabCover :prefab="prefab" class="relative h-full shadow-card">
    <!-- Voile sous le texte : il reste lisible sur une bannière chargée. -->
    <div class="pointer-events-none absolute h-100 inset-0 bg-gradient-to-r from-card/95 via-card/80 to-card/40" aria-hidden="true" />
    <div class="relative flex h-full gap-4 p-5" :class="thumbnail ? 'pt-5' : 'pt-16'">
      <img
        v-if="thumbnail"
        :src="thumbnail"
        alt=""
        data-slot="prefab-thumbnail"
        class="aspect-[3/4] w-24 shrink-0 self-start rounded-lg object-cover shadow-card ring-2 ring-card"
      >
      <div class="grid min-w-0 flex-1 content-start gap-3">
        <div class="flex items-center gap-3">
          <PrefabIcon :prefab="prefab" :class="thumbnail ? 'size-9 rounded-lg' : 'size-12 rounded-xl text-lg ring-2 ring-card'" />
          <div class="grid min-w-0">
            <p class="truncate text-[15px] font-bold">{{ name }}</p>
            <p class="truncate font-mono text-xs text-muted-foreground">{{ `${prefab.id} · ${prefab.version}` }}</p>
          </div>
        </div>
        <p v-if="description" class="line-clamp-2 text-[13px] text-muted-foreground">{{ description }}</p>
        <div class="flex flex-wrap items-center gap-2">
          <Badge variant="secondary">{{ t(`prefabs.catalog.categories.${prefab.category}`) }}</Badge>
          <Badge :variant="prefab.source === 'official' ? 'info' : 'default'">{{ t(`prefabs.catalog.${prefab.source}`) }}</Badge>
          <PrefabBlockedBadge :prefab="prefab" />
        </div>
        <p class="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span>{{ t('prefabs.catalog.versions', versions) }}</span>
          <span class="inline-flex items-center gap-1"><Server class="size-3.5" />{{ t('prefabs.catalog.servers', servers) }}</span>
        </p>
      </div>
    </div>
  </PrefabCover>
</template>
