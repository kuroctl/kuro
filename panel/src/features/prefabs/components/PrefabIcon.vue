<script setup lang="ts">
// Icône d'un prefab ; à défaut, son initiale sur un dégradé propre à son id.
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import { cn } from '@/shared/lib/utils'

import { useCatalogEntry, usePrefabAsset } from '../prefabs.queries'
import { prefabHue } from './prefabHue'

const props = defineProps<{ prefab: { id: string; version: string } | undefined; class?: string }>()

const { locale } = useI18n()
const entry = useCatalogEntry(() => props.prefab)
const src = usePrefabAsset(() => props.prefab, 'icon')

const initial = computed(() => {
  const names = entry.value?.names ?? {}
  const name = names[locale.value] ?? names.en ?? props.prefab?.id ?? '?'
  return name.charAt(0).toUpperCase()
})
const fallback = computed(() => {
  const hue = prefabHue(props.prefab?.id ?? '')
  return { background: `linear-gradient(135deg, oklch(0.62 0.17 ${hue}), oklch(0.52 0.18 ${hue + 40}))` }
})
</script>

<template>
  <span
    aria-hidden="true"
    :class="cn('inline-grid size-8 shrink-0 place-items-center overflow-hidden rounded-lg text-sm font-bold text-white', props.class)"
    :style="src ? undefined : fallback"
  >
    <img v-if="src" :src="src" alt="" class="size-full object-cover">
    <template v-else>{{ initial }}</template>
  </span>
</template>
