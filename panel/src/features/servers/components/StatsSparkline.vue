<script setup lang="ts">
// Courbe minimale (SVG) d'une ou plusieurs séries, de 0 à `max` (ou au plus haut point).
import { computed } from 'vue'

const props = defineProps<{
  series: number[][]
  /** Fenêtre en nombre de points : la courbe se remplit de droite à gauche. */
  window: number
  max?: number
}>()

const WIDTH = 100
const HEIGHT = 32
const tones = ['stroke-primary', 'stroke-muted-foreground']

const paths = computed(() => {
  const top = props.max ?? Math.max(1, ...props.series.flat())
  const step = WIDTH / Math.max(1, props.window - 1)
  return props.series.map((values) => {
    const offset = props.window - values.length
    return values
      .map((value, index) => {
        const x = (offset + index) * step
        const y = HEIGHT - (Math.min(value, top) / top) * (HEIGHT - 2) - 1
        return `${x.toFixed(2)},${y.toFixed(2)}`
      })
      .join(' ')
  })
})
</script>

<template>
  <svg :viewBox="`0 0 ${WIDTH} ${HEIGHT}`" preserveAspectRatio="none" class="h-8 w-full" aria-hidden="true">
    <polyline
      v-for="(points, index) in paths"
      :key="index"
      :points="points"
      fill="none"
      stroke-width="1.5"
      vector-effect="non-scaling-stroke"
      :class="tones[index % tones.length]"
    />
  </svg>
</template>
