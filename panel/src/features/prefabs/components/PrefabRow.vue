<script setup lang="ts">
// Ligne d'un prefab en vue liste : icône, nom, description, puis catégorie, version et serveurs
// selon la largeur. Contenu seul : le parent l'enveloppe d'un lien ou d'un bouton.
import { ChevronRight, Server } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import type { Schemas } from '@/shared/api/types'
import { Badge } from '@/shared/ui/badge'

import PrefabBlockedBadge from './PrefabBlockedBadge.vue'
import PrefabIcon from './PrefabIcon.vue'

const props = defineProps<{ prefab: Schemas['PrefabSummaryDto']; servers: number; versions: number }>()
const { t, locale } = useI18n()

const name = computed(() => props.prefab.names[locale.value] ?? props.prefab.names.en ?? props.prefab.id)
const description = computed(() => props.prefab.descriptions[locale.value] ?? props.prefab.descriptions.en)
</script>

<template>
  <span class="flex w-full items-center gap-3 px-4 py-3">
    <PrefabIcon :prefab="prefab" class="size-10 rounded-lg" />
    <span class="grid min-w-0 flex-1 gap-0.5">
      <span class="flex min-w-0 items-center gap-2">
        <span class="truncate font-semibold">{{ name }}</span>
        <PrefabBlockedBadge :prefab="prefab" />
      </span>
      <span class="truncate text-[13px] text-muted-foreground">{{ description }}</span>
    </span>
    <Badge variant="secondary" class="hidden shrink-0 sm:inline-flex">{{ t(`prefabs.catalog.categories.${prefab.category}`) }}</Badge>
    <span class="hidden w-44 shrink-0 text-right text-xs text-muted-foreground md:grid">
      <span class="truncate font-mono">{{ `${prefab.id} · ${prefab.version}` }}</span>
      <span>{{ t('prefabs.catalog.versions', versions) }}</span>
    </span>
    <span class="hidden w-24 shrink-0 items-center justify-end gap-1 text-xs text-muted-foreground lg:inline-flex" :title="t('prefabs.catalog.servers', servers)">
      <Server class="size-3.5" />{{ servers }}
    </span>
    <ChevronRight class="size-4 shrink-0 text-muted-foreground" />
  </span>
</template>
