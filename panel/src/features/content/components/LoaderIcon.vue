<script setup lang="ts">
// Icône d'un loader (Fabric, Paper…) fournie par la source de contenu. Le SVG vient d'un tiers :
// il sert seulement de masque CSS, coloré par le texte, et n'est jamais inséré dans la page.
import { computed } from 'vue'

import { cn } from '@/shared/lib/utils'

import { useContentLoaders } from '../content.queries'

const props = defineProps<{ name: string | null | undefined; class?: string }>()

const { data: loaders } = useContentLoaders(() => Boolean(props.name))
const mask = computed(() => {
  const svg = loaders.value?.find((loader) => loader.name === props.name)?.icon_svg
  if (!svg) return undefined
  const url = `url("data:image/svg+xml,${encodeURIComponent(svg)}")`
  return { maskImage: url, WebkitMaskImage: url }
})
</script>

<template>
  <span
    v-if="mask"
    aria-hidden="true"
    :class="cn('inline-block size-4 shrink-0 bg-current [mask-position:center] [mask-repeat:no-repeat] [mask-size:contain]', props.class)"
    :style="mask"
  />
</template>
