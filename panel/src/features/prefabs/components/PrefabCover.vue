<script setup lang="ts">
// Bannière d'un prefab ; à défaut, un dégradé propre à son id. Le contenu (nom, état…) est posé
// par-dessus, sur un voile qui garde le texte lisible quelle que soit l'image.
import { computed } from 'vue'

import { cn } from '@/shared/lib/utils'

import { usePrefabAsset } from '../prefabs.queries'
import { prefabHue } from './prefabHue'

const props = defineProps<{ prefab: { id: string; version: string } | undefined; class?: string }>()

const src = usePrefabAsset(() => props.prefab, 'cover')
const fallback = computed(() => {
  const hue = prefabHue(props.prefab?.id ?? '')
  return {
    background: `radial-gradient(120% 140% at 0% 0%, oklch(0.55 0.16 ${hue} / 55%), transparent 60%), radial-gradient(100% 120% at 100% 100%, oklch(0.5 0.17 ${hue + 50} / 45%), transparent 55%)`,
  }
})
</script>

<template>
  <div :class="cn('relative overflow-hidden rounded-xl border bg-card', props.class)">
    <img v-if="src" :src="src" alt="" aria-hidden="true" class="absolute inset-0 size-full object-cover">
    <div v-else class="absolute inset-0" :style="fallback" aria-hidden="true" />
    <div class="absolute inset-0 bg-gradient-to-t from-card via-card/70 to-card/10" aria-hidden="true" />
    <div class="relative"><slot /></div>
  </div>
</template>
